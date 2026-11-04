//! Bounded, positive proofs that refine findings without hiding unknown paths.
//! Factory summaries come from the resolved resource graph, never package names.

mod factory;
mod facts;
mod proofs;

use super::builtins;
use crate::model::{DependencyKind, Polyfill, PolyfillGuard, Rule, ScriptMode, Span};
use crate::scheduler::{Cancellation, Resource};
use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::*};
use oxc_parser::Parser;
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::{node::NodeId, symbol::SymbolId};
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq)]
enum Fact {
    Unknown,
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Object(BTreeMap<String, Fact>),
    Native(String),
    /// Either this native reference or null/undefined. A property read throws
    /// on the nullish arm, so a completed read still has the native identity.
    MaybeNative(String),
}

impl Fact {
    fn truth(&self) -> Option<bool> {
        match self {
            Self::Undefined | Self::Null => Some(false),
            Self::Bool(b) => Some(*b),
            Self::Number(n) => Some(*n != 0.0 && !n.is_nan()),
            Self::String(s) => Some(!s.is_empty()),
            Self::Object(_) => Some(true),
            // A known global path does not establish that the API exists.
            _ => None,
        }
    }

    fn nullish(&self) -> Option<bool> {
        match self {
            Self::Undefined | Self::Null => Some(true),
            Self::Unknown | Self::Native(_) | Self::MaybeNative(_) => None,
            _ => Some(false),
        }
    }
}

#[derive(Clone)]
struct Factory {
    callback: usize,
    arguments: Vec<Fact>,
}

struct Proof<'s, 'a> {
    semantic: &'s Semantic<'a>,
    definitions: HashMap<SymbolId, &'a Expression<'a>>,
    parameters: HashMap<SymbolId, Fact>,
    budget: Cell<usize>,
    exhausted: Cell<bool>,
    cancel: &'s Cancellation,
}

impl<'s, 'a> Proof<'s, 'a> {
    fn new(semantic: &'s Semantic<'a>, max_steps: usize, cancel: &'s Cancellation) -> Self {
        let definitions = semantic
            .nodes()
            .iter_enumerated()
            .filter_map(|(_, n)| {
                if let AstKind::VariableDeclarator(d) = n.kind() {
                    if let (BindingPattern::BindingIdentifier(i), Some(e)) = (&d.id, &d.init) {
                        return Some((i.symbol_id.get()?, e));
                    }
                }
                None
            })
            .collect();
        Self {
            semantic,
            definitions,
            parameters: HashMap::new(),
            budget: Cell::new(max_steps),
            exhausted: Cell::new(false),
            cancel,
        }
    }

    fn step(&self) -> bool {
        let Some(next) = self.budget.get().checked_sub(1) else {
            self.exhausted.set(true);
            return false;
        };
        self.budget.set(next);
        if next % 256 == 0 && self.cancel.check().is_err() {
            self.exhausted.set(true);
            return false;
        }
        true
    }

    fn symbol(&self, i: &IdentifierReference<'_>) -> Option<SymbolId> {
        i.reference_id
            .get()
            .and_then(|r| self.semantic.scoping().get_reference(r).symbol_id())
    }
}

fn with_proof<T>(
    resource: &Resource,
    max_steps: usize,
    cancel: &Cancellation,
    run: impl FnOnce(&mut Proof<'_, '_>) -> T,
) -> Option<T> {
    if cancel.check().is_err() {
        return None;
    }
    let analysis = resource.analysis.as_ref()?;
    if analysis.gaps.iter().any(|g| {
        matches!(
            g.reason.as_str(),
            "parse-error" | "step-budget" | "time-budget" | "cancelled" | "with-statement"
        )
    }) {
        return None;
    }
    let source = &resource.loaded.as_ref()?.content;
    let ty = if resource.mode == Some(ScriptMode::Module) {
        SourceType::mjs().with_jsx(true)
    } else {
        SourceType::cjs().with_script(true)
    };
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, ty).parse();
    if parsed.fatal_error || parsed.diagnostics.has_errors() {
        return None;
    }
    let built = SemanticBuilder::new()
        .with_build_nodes(true)
        .build(&parsed.program);
    if built.semantic.nodes().len() > max_steps || cancel.check().is_err() {
        return None;
    }
    let mut proof = Proof::new(&built.semantic, max_steps, cancel);
    if proof.semantic.nodes().iter_enumerated().any(|(_, n)| matches!(n.kind(), AstKind::CallExpression(c) if matches!(c.callee.get_inner_expression(), Expression::Identifier(i) if i.name=="eval" && proof.symbol(i).is_none()))) { return None; }
    let result = run(&mut proof);
    (!proof.exhausted.get() && cancel.check().is_ok()).then_some(result)
}

pub(crate) fn prune(resources: &mut [Resource], max_steps: usize, cancel: &Cancellation) {
    let summaries: Vec<_> = resources
        .iter()
        .map(|r| with_proof(r, max_steps, cancel, |p| p.exports()).unwrap_or_default())
        .collect();
    for index in 0..resources.len() {
        if cancel.check().is_err() {
            return;
        }
        let resource = &resources[index];
        if resource
            .analysis
            .as_ref()
            .is_none_or(|a| a.findings.is_empty())
        {
            continue;
        }
        let changes = with_proof(resource, max_steps, cancel, |p| {
            let mut factories = p.local_factories();
            let targets: HashMap<_, _> = resource
                .edges
                .iter()
                .filter_map(|edge| {
                    let dependency = resource
                        .analysis
                        .as_ref()?
                        .dependencies
                        .get(edge.dependency)?;
                    (dependency.kind == DependencyKind::Import)
                        .then_some((dependency.span.start, edge.to?))
                })
                .collect();
            for (_, n) in p.semantic.nodes().iter_enumerated() {
                if let AstKind::ImportDeclaration(i) = n.kind() {
                    let Some(target) = targets.get(&i.span.start) else {
                        continue;
                    };
                    for specifier in i.specifiers.iter().flatten() {
                        if let ImportDeclarationSpecifier::ImportSpecifier(s) = specifier {
                            if let (Some(symbol), Some(factory)) = (
                                s.local.symbol_id.get(),
                                summaries[*target].get(s.imported.name().as_str()),
                            ) {
                                factories.insert(symbol, factory.clone());
                            }
                        }
                    }
                }
            }
            p.bind_factories(&factories);
            p.changes()
        });
        let Some((dead, remove, polyfills)) = changes else {
            continue;
        };
        if let Some(analysis) = &mut resources[index].analysis {
            analysis.findings.retain_mut(|f| {
                if dead
                    .iter()
                    .any(|s| s.start <= f.span.start && f.span.end <= s.end)
                    || remove.contains(&(f.rule, f.span))
                {
                    return false;
                }
                if let Some((target, guard)) = polyfills.get(&f.span) {
                    if matches!(f.rule, Rule::BuiltinMutation | Rule::HostApiOverride) {
                        f.target = target.clone();
                        f.polyfill = Some(Polyfill {
                            guard: PolyfillGuard::Missing,
                            guard_target: guard.clone(),
                        });
                    }
                }
                true
            });
        }
    }
}
