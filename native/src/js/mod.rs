//! JavaScript pollution analysis.
//!
//! One `Analyzer` handles one resource. It parses with Oxc, builds scopes and
//! symbol references, then walks the AST once. During the walk it:
//!
//! * evaluates a small abstract value for expressions that matter
//!   (`eval.rs`): references to the global object, `document`, builtins,
//!   script-created DOM nodes and finite strings,
//! * applies the pollution rules to assignments, calls, declarations and
//!   deletes (`rules.rs`),
//! * records dependencies (`import`, `import()`, workers, dynamic scripts),
//! * records gaps for constructs it cannot see through (`eval`, `with`,
//!   unknown dynamic URLs, budget exhaustion).
//!
//! The analyzer never executes code. Abstract global-object evaluation follows
//! immediately-invoked function arguments so UMD/bundler wrappers resolve.
//! Module-state exclusions additionally inspect bounded helper-call summaries
//! for cleanup pairs, successful module-loading results and initialization-only
//! calls across the resolved resource graph.

mod builtins;
mod cleanup_pairs;
mod eval;
mod module_state;
mod polyfill;
mod rules;
mod static_filter;
mod values;
mod visitor;

pub(crate) use module_state::initialization::prune as prune_initialization;
pub(crate) use static_filter::prune as prune_static;

use crate::model::{
    Analysis, Confidence, Dependency, DependencyKind, Finding, Gap, Rule, ScriptMode, Span,
};
use crate::scheduler::Cancellation;
use oxc_allocator::Allocator;
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::SourceType;
use oxc_syntax::symbol::SymbolId;
use std::collections::{HashMap, HashSet};
use values::Value;

/// Per-resource analysis configuration.
#[derive(Clone, Debug)]
pub struct JsOptions {
    pub script_mode: ScriptMode,
    /// The resource's own URL, used for `import.meta.url` and worker bases.
    pub resource_url: String,
    /// Document base for inline scripts (workers and dynamic scripts resolve
    /// against the document, not the inline resource identity).
    pub base_url: Option<String>,
    pub max_steps: usize,
    pub max_candidates: usize,
}

/// Stable version string for the parser, reported in manifests.
pub const PARSER_VERSION: &str = "oxc-0.154.0";

/// Parses and analyzes one JavaScript resource.
pub fn analyze(
    source: &str,
    options: &JsOptions,
    cancel: &Cancellation,
    map: Option<&sourcemap::DecodedMap>,
) -> Analysis {
    let source_type = match options.script_mode {
        ScriptMode::Module => SourceType::mjs().with_jsx(true),
        ScriptMode::Classic => SourceType::cjs().with_script(true),
    };
    let mut analysis = analyze_as(source, source_type, options, cancel);
    module_state::refine(&mut analysis, source, map, options, cancel);
    module_state::focus::retain(&mut analysis, source, source_type, map, options, cancel);
    cleanup_pairs::prune(&mut analysis, source, source_type, options, cancel, map);
    analysis
}

fn analyze_as(
    source: &str,
    source_type: SourceType,
    options: &JsOptions,
    cancel: &Cancellation,
) -> Analysis {
    analyze_as_with_sources(source, source_type, options, cancel, None)
}

fn analyze_as_with_sources(
    source: &str,
    source_type: SourceType,
    options: &JsOptions,
    cancel: &Cancellation,
    sources: Option<(&sourcemap::DecodedMap, &str)>,
) -> Analysis {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    let mut analysis = Analysis::default();
    if parsed.fatal_error || parsed.diagnostics.has_errors() {
        let detail = parsed
            .diagnostics
            .errors()
            .next()
            .map(|d| d.message.to_string())
            .unwrap_or_else(|| "parse-error".into());
        analysis.gaps.push(Gap {
            reason: "parse-error".into(),
            span: Span::new(0, source.len() as u32),
            detail: Some(detail),
        });
        return analysis;
    }
    let semantic = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(false)
        .build(&parsed.program);
    let scoping = semantic.semantic.scoping();
    let mut analyzer = Analyzer::new(scoping, options, cancel);
    analyzer.visit_program(&parsed.program);
    let mut analysis = analyzer.finish();
    module_state::prune_patterns(&mut analysis, &semantic.semantic, options, cancel, sources);
    analysis
}

/// A finding whose validity depends on later statements in the same file
/// (style injections and listeners can be undone by `remove()` /
/// `removeEventListener`).
struct Deferred {
    finding: Finding,
    node: Option<u32>,
    listener: Option<ListenerKey>,
    block: u32,
    function_scope: u32,
    unconditional: bool,
    /// Binding of the local function this effect was recorded in, when it
    /// has one; used to substitute helper parameters at call sites.
    owner: Option<SymbolId>,
}

/// Identity of a callback (or other argument) reference: a binding, or a
/// member path such as `this.visibilityHandler` (rooted at a binding when
/// resolvable).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Callback {
    Binding(SymbolId),
    Member(String),
}

/// Event component of a listener key: a known name, or the reference that
/// carries it in generic registration helpers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EventKey {
    Literal(String),
    Ref(Callback),
}

/// One argument at a call to a local helper, used to substitute the
/// helper's parameters in its deferred listener keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ArgKey {
    Literal(String),
    Ref(Callback),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ListenerKey {
    event: EventKey,
    callback: Callback,
}

impl ListenerKey {
    /// File-level suppression uses event and handler identity only.
    /// Receiver, capture, source order and conditions do not participate.
    fn pairs_with(&self, other: &Self) -> bool {
        self.event == other.event && self.callback == other.callback
    }
}

/// A `removeEventListener` seen anywhere in the file, kept for
/// end-of-file pairing with registrations in other functions (for
/// example an attach/detach pair).
struct ListenerRemoval {
    key: ListenerKey,
    owner: Option<SymbolId>,
}

pub(crate) struct Analyzer<'s, 'c> {
    scoping: &'s Scoping,
    options: &'c JsOptions,
    cancel: &'c Cancellation,
    /// Abstract values of bindings, keyed by Oxc symbol id.
    values: HashMap<SymbolId, Value>,
    /// Value of `this` for the innermost non-arrow function. The top of the
    /// stack is the global object in classic scripts.
    this_stack: Vec<Value>,
    /// Set while evaluating when the result passed through a reassignable
    /// binding or a joined branch, lowering confidence of the finding.
    uncertain: bool,
    /// Whether the root scope is strict (`"use strict"` or module).
    strict: bool,
    findings: Vec<Finding>,
    polyfill_guards: Vec<polyfill::Guard>,
    deferred: Vec<Deferred>,
    /// Node allocation id -> removal sites (scope, offset, known parent targets).
    /// Callback definitions can precede insertion, so DOM pairing is finalized
    /// after the entire resource has been visited.
    node_removals: HashMap<u32, Vec<(u32, u32, Option<Vec<String>>)>>,
    listener_removals: Vec<ListenerRemoval>,
    /// Current synchronous execution scope; callback/function bodies get their
    /// own identity, while direct IIFEs retain their caller's scope.
    function_scope: u32,
    /// Current block for immediate node/style cleanup.
    block: u32,
    /// Spans (start offsets) of function literals that form a `return`
    /// value, e.g. an unsubscribe/effect cleanup function.
    returned_fns: HashSet<u32>,
    /// Blocks of enclosing functions that returned the function currently
    /// being walked. Removals inside a returned cleanup function may cancel
    /// deferred findings recorded in those blocks.
    cleanup_blocks: Vec<u32>,
    /// Canonical reference identity of a binding assigned from another
    /// reference (`const g = f`, `const h = this.handler`).
    aliases: HashMap<SymbolId, Callback>,
    /// Bindings assigned from `this`, so `t.handler` keys as `this.handler`.
    this_aliases: HashSet<SymbolId>,
    /// Parameter bindings and names of local functions, by function binding.
    fn_params: HashMap<SymbolId, Vec<Option<(SymbolId, String)>>>,
    /// Declaration spans of module bindings initialized with function literals.
    module_functions: HashSet<Span>,
    /// Recorded calls to local functions: binding and argument identities.
    fn_calls: Vec<(SymbolId, Vec<Option<ArgKey>>)>,
    /// Owner binding of each enclosing function, innermost last.
    fn_stack: Vec<Option<SymbolId>>,
    /// Owner binding for a function literal currently being visited as a
    /// variable initializer or assignment value.
    pending_owner: Vec<Option<SymbolId>>,
    /// Enclosing write expression for module-state target locations.
    write_context: Option<(Span, &'static str)>,
    conditional_depth: usize,
    dependencies: Vec<Dependency>,
    gaps: Vec<Gap>,
    /// Deduplicates findings by (rule, target, span).
    seen: HashSet<(Rule, String, Span)>,
    steps: usize,
    /// Set when the step budget or deadline is hit; stops the walk.
    exhausted: bool,
}

impl<'s, 'c> Analyzer<'s, 'c> {
    fn new(scoping: &'s Scoping, options: &'c JsOptions, cancel: &'c Cancellation) -> Self {
        let strict = options.script_mode == ScriptMode::Module
            || scoping.root_scope_flags().is_strict_mode();
        let this_root = match options.script_mode {
            ScriptMode::Classic => Value::Targets(vec![values::Target::new(values::Root::Global)]),
            ScriptMode::Module => Value::Undefined,
        };
        Self {
            scoping,
            options,
            cancel,
            values: HashMap::new(),
            this_stack: vec![this_root],
            uncertain: false,
            strict,
            findings: vec![],
            polyfill_guards: vec![],
            deferred: vec![],
            node_removals: HashMap::new(),
            listener_removals: vec![],
            function_scope: 0,
            block: 0,
            returned_fns: HashSet::new(),
            cleanup_blocks: vec![],
            aliases: HashMap::new(),
            this_aliases: HashSet::new(),
            fn_params: HashMap::new(),
            module_functions: HashSet::new(),
            fn_calls: vec![],
            fn_stack: vec![],
            pending_owner: vec![],
            write_context: None,
            conditional_depth: 0,
            dependencies: vec![],
            gaps: vec![],
            seen: HashSet::new(),
            steps: 0,
            exhausted: false,
        }
    }

    /// Counts one AST step; returns `false` once the budget is exhausted so
    /// visitors can stop descending.
    fn step(&mut self, span: Span) -> bool {
        if self.exhausted {
            return false;
        }
        self.steps += 1;
        if self.steps > self.options.max_steps {
            self.exhausted = true;
            self.gap("step-budget", span, None);
            return false;
        }
        if self.steps % 1024 == 0 {
            if let Err(reason) = self.cancel.check() {
                self.exhausted = true;
                self.gap(reason, span, None);
                return false;
            }
        }
        true
    }

    fn gap(&mut self, reason: &'static str, span: Span, detail: Option<String>) {
        if !self
            .gaps
            .iter()
            .any(|g| g.reason == reason && g.span == span)
        {
            self.gaps.push(Gap {
                reason: reason.into(),
                span,
                detail,
            });
        }
    }

    fn finding(
        &mut self,
        rule: Rule,
        span: Span,
        target: String,
        operation: &str,
        detail: Option<String>,
    ) {
        let confidence = if self.uncertain || target.contains('*') {
            Confidence::Medium
        } else {
            Confidence::High
        };
        if self.seen.insert((rule, target.clone(), span)) {
            self.findings.push(Finding {
                rule,
                span,
                target,
                operation: operation.into(),
                confidence,
                detail,
                polyfill: None,
                declaration: None,
            });
        }
    }

    fn deferred_finding(
        &mut self,
        rule: Rule,
        span: Span,
        target: String,
        operation: &str,
        detail: Option<String>,
        node: Option<u32>,
        listener: Option<ListenerKey>,
    ) {
        let confidence = if self.uncertain || target.contains('*') {
            Confidence::Medium
        } else {
            Confidence::High
        };
        if self.seen.insert((rule, target.clone(), span)) {
            self.deferred.push(Deferred {
                finding: Finding {
                    rule,
                    span,
                    target,
                    operation: operation.into(),
                    confidence,
                    detail,
                    polyfill: None,
                    declaration: None,
                },
                node,
                listener,
                block: self.block,
                function_scope: self.function_scope,
                unconditional: self.conditional_depth == 0,
                owner: self.fn_stack.last().copied().flatten(),
            });
        }
    }

    fn cleanup(&mut self, node: u32, span: Span, parents: Option<Vec<String>>) {
        self.node_removals.entry(node).or_default().push((
            self.function_scope,
            span.start,
            parents.clone(),
        ));
        // Preserve immediate/returned cleanup for stylesheet injections. DOM
        // insertions can also pair with callback removals at end of file.
        if self.conditional_depth != 0 {
            return;
        }
        self.deferred.retain(|d| {
            let same = d.node == Some(node)
                && parents
                    .as_ref()
                    .is_none_or(|parents| parents.contains(&d.finding.target));
            // A removal inside a function returned from the registering
            // block pairs with registrations on that same execution path,
            // even when both sit under a branch (for example an effect
            // that only subscribes while visible).
            let returned_cleanup = self.cleanup_blocks.contains(&d.block);
            let same_block = d.block == self.block || returned_cleanup;
            !(same && (returned_cleanup || (d.unconditional && same_block)))
        });
    }

    fn dependency(
        &mut self,
        kind: DependencyKind,
        span: Span,
        value: &Value,
        mode: Option<ScriptMode>,
    ) {
        let specifiers = value.strings();
        if specifiers.is_empty() {
            let reason = match kind {
                DependencyKind::DynamicImport => "dynamic-import-specifier",
                DependencyKind::Worker => "dynamic-worker-url",
                DependencyKind::DynamicScript => "dynamic-script-url",
                DependencyKind::DynamicStyle => "dynamic-style-url",
                _ => "dynamic-dependency",
            };
            self.gap(reason, span, None);
            return;
        }
        let base_url = match kind {
            // Workers and DOM-loaded resources resolve against the document,
            // which for inline scripts differs from the resource identity.
            DependencyKind::Worker
            | DependencyKind::DynamicScript
            | DependencyKind::DynamicStyle => self.options.base_url.clone(),
            _ if self.options.resource_url.contains("#inline@") => self.options.base_url.clone(),
            _ => None,
        };
        self.dependencies.push(Dependency {
            kind,
            span,
            specifiers,
            content: None,
            content_offset: None,
            script_mode: mode,
            base_url,
        });
    }

    /// Resolves deferred findings against the removals seen in the file.
    ///
    /// A listener registration is cancelled by a `removeEventListener`
    /// with a pairing key anywhere in the file, regardless of definition
    /// order or the branches around either call. A registration recorded
    /// inside a local helper also pairs through its call sites: helper
    /// parameters in the key are substituted with each call's argument
    /// identities, and one paired call is enough to cancel the helper's
    /// single finding.
    fn finish(mut self) -> Analysis {
        // Classify after the walk so declaration order does not affect exclusion.
        self.findings.retain(|finding| {
            !finding
                .declaration
                .as_ref()
                .is_some_and(|declaration| self.module_functions.contains(&declaration.span))
        });
        let removals = std::mem::take(&mut self.listener_removals);
        let node_removals = std::mem::take(&mut self.node_removals);
        let fn_params = std::mem::take(&mut self.fn_params);
        let fn_calls = std::mem::take(&mut self.fn_calls);
        let keys_at_calls = |key: &ListenerKey, owner: Option<SymbolId>| {
            let mut keys = vec![key.clone()];
            if let Some((owner, params)) = owner.and_then(|o| fn_params.get(&o).map(|p| (o, p))) {
                keys.extend(
                    fn_calls
                        .iter()
                        .filter(|(f, _)| *f == owner)
                        .filter_map(|(_, args)| substitute_key(key, params, args)),
                );
            }
            keys
        };
        let removal_keys: Vec<ListenerKey> = removals
            .iter()
            .flat_map(|r| keys_at_calls(&r.key, r.owner))
            .collect();
        self.deferred.retain(|d| {
            if d.finding.rule == Rule::DomInsertion
                && d.node
                    .and_then(|node| node_removals.get(&node))
                    .is_some_and(|sites| {
                        sites.iter().any(|(scope, start, parents)| {
                            (*scope != d.function_scope || *start > d.finding.span.start)
                                && parents
                                    .as_ref()
                                    .is_none_or(|parents| parents.contains(&d.finding.target))
                        })
                    })
            {
                return false;
            }
            let Some(key) = &d.listener else { return true };
            !keys_at_calls(key, d.owner)
                .iter()
                .any(|k| removal_keys.iter().any(|r| k.pairs_with(r)))
        });
        self.findings
            .extend(self.deferred.into_iter().map(|d| d.finding));
        self.findings.sort_by_key(|f| (f.span.start, f.span.end));
        Analysis {
            findings: self.findings,
            dependencies: self.dependencies,
            gaps: self.gaps,
            base_url: None,
            import_map: None,
        }
    }
}

/// Argument identity for a helper parameter, when the key component is
/// exactly that parameter binding.
fn param_arg<'a>(
    callback: &Callback,
    params: &[Option<(SymbolId, String)>],
    args: &'a [Option<ArgKey>],
) -> Option<Option<&'a ArgKey>> {
    let position = match callback {
        Callback::Binding(symbol) => params
            .iter()
            .position(|p| p.as_ref().is_some_and(|(s, _)| s == symbol)),
        Callback::Member(_) => None,
    };
    position.map(|index| args.get(index).and_then(|a| a.as_ref()))
}

fn substitute_ref(
    callback: &Callback,
    params: &[Option<(SymbolId, String)>],
    args: &[Option<ArgKey>],
) -> Option<Callback> {
    match param_arg(callback, params, args) {
        None => Some(callback.clone()),
        Some(Some(ArgKey::Ref(r))) => Some(r.clone()),
        _ => None,
    }
}

/// A helper's listener key with its parameter components replaced by one
/// call site's argument identities.
fn substitute_key(
    key: &ListenerKey,
    params: &[Option<(SymbolId, String)>],
    args: &[Option<ArgKey>],
) -> Option<ListenerKey> {
    let event = match &key.event {
        EventKey::Literal(_) => key.event.clone(),
        EventKey::Ref(r) => match param_arg(r, params, args) {
            None => key.event.clone(),
            Some(Some(ArgKey::Literal(s))) => EventKey::Literal(s.clone()),
            Some(Some(ArgKey::Ref(c))) => EventKey::Ref(c.clone()),
            _ => return None,
        },
    };
    Some(ListenerKey {
        event,
        callback: substitute_ref(&key.callback, params, args)?,
    })
}

#[cfg(test)]
#[path = "../tests/js.rs"]
mod tests;
