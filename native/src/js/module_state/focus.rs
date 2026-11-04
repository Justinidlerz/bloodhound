//! Positive evidence for state inherited by a new component instance.
//! Unclassified shared state is outside this rule, not established to be safe.

mod flow;
mod receiver;
mod recognize;

use crate::js::JsOptions;
use crate::lines::LineIndex;
use crate::model::{Analysis, Confidence, Gap, Rule, Span};
use crate::scheduler::Cancellation;
use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::*};
use oxc_parser::Parser;
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::{node::NodeId, symbol::SymbolId};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

type Evidence = HashMap<(u32, u32), (&'static str, Confidence)>;
type Environment<'a> = HashMap<SymbolId, &'a Expression<'a>>;

const INITIAL_STATE: &str = "Module state flows into a React state initializer and is written from the same component or its local callbacks/helpers. No dominating initialization or paired effect cleanup was resolved. A later mount can inherit the previous value; this is static evidence, not a runtime defect confirmation.";
const SKIPPED_STATE: &str = "A retained module value can return early from a component lifecycle callback or local helper before a resolved React state update. No preceding refresh or paired effect cleanup was resolved. A later instance can skip its state update; this is static evidence, not a runtime defect confirmation.";
const SKIPPED_RECEIVER_STATE: &str = "A retained module value can return early before a receiver method whose name matches a unique state-and-payload merge reducer in the same factory descriptor. No preceding refresh or complete temporary-guard reset was resolved. If the factory supplies a new state instance on remount, the retained guard can skip refilling it. Receiver dispatch and instance lifetime require verification; this is a structural candidate, not a runtime defect confirmation.";

pub(crate) fn retain(
    analysis: &mut Analysis,
    source: &str,
    source_type: SourceType,
    map: Option<&sourcemap::DecodedMap>,
    options: &JsOptions,
    cancel: &Cancellation,
) {
    if !analysis
        .findings
        .iter()
        .any(|f| f.rule == Rule::ModuleStateWrite)
    {
        return;
    }
    let lines = LineIndex::new(source);
    let mut generated = None;
    let mut originals = HashMap::new();
    let mut incomplete = false;
    analysis.findings.retain_mut(|finding| {
        if finding.rule != Rule::ModuleStateWrite {
            return true;
        }
        let Some(declaration) = &finding.declaration else {
            return false;
        };
        // Object mutations require a property-sensitive flow, not merely a
        // shared root binding. Keep this pass limited to scalar assignments.
        if finding.target.contains(['.', '[']) {
            return false;
        }
        let point = lines.position(finding.span.start);
        let token = map.and_then(|m| m.lookup_token(point.0 - 1, point.1));
        let original = token
            .as_ref()
            .and_then(|t| Some((t.get_source()?, t.get_source_view()?)));
        let (result, point) = if let (Some((name, view)), Some(location)) =
            (original, declaration.original.as_ref())
        {
            let result = originals
                .entry((name, view.source().as_ptr()))
                .or_insert_with(|| {
                    let ty = SourceType::from_path(name)
                        .unwrap_or_else(|_| SourceType::mjs().with_jsx(true))
                        .with_module(true);
                    collect(view.source(), ty, options, cancel)
                });
            (result.as_ref(), (location.line, location.column))
        } else {
            let result =
                generated.get_or_insert_with(|| collect(source, source_type, options, cancel));
            (result.as_ref(), lines.position(declaration.span.start))
        };
        let Some(result) = result else {
            incomplete = true;
            return false;
        };
        let Some((detail, confidence)) = result.get(&point) else {
            return false;
        };
        finding.detail = Some((*detail).into());
        // Confidence describes resolved data flow, not certainty of a bug.
        finding.confidence = *confidence;
        true
    });
    if incomplete && cancel.check().is_ok() {
        analysis.gaps.push(Gap {
            reason: "module-state-analysis-incomplete".into(),
            span: Span::new(0, source.len() as u32),
            detail: Some("Positive component-state analysis could not finish parsing or exceeded its analysis budget. Unconfirmed module-state writes are not reported.".into()),
        });
    }
}

fn collect(
    source: &str,
    ty: SourceType,
    options: &JsOptions,
    cancel: &Cancellation,
) -> Option<Evidence> {
    if cancel.check().is_err() {
        return None;
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, ty).parse();
    if parsed.fatal_error || parsed.diagnostics.has_errors() {
        return None;
    }
    let built = SemanticBuilder::new()
        .with_build_nodes(true)
        .build(&parsed.program);
    let semantic = &built.semantic;
    if semantic.nodes().len() > options.max_steps {
        return None;
    }
    let mut flow = Flow {
        semantic,
        definitions: HashMap::new(),
        functions: HashMap::new(),
        imports: HashMap::new(),
        setters: HashMap::new(),
        receiver_setters: HashMap::new(),
        candidates: HashSet::new(),
        calls: Vec::new(),
        budget: Cell::new(options.max_steps),
        exhausted: Cell::new(false),
        cancel,
    };
    flow.prepare();
    flow.prepare_receivers();
    let selected = flow.recognize();
    if flow.exhausted.get() || cancel.check().is_err() {
        return None;
    }
    let lines = LineIndex::new(source);
    Some(
        selected
            .into_iter()
            .map(|(s, detail)| {
                (
                    lines.position(semantic.scoping().symbol_span(s).start),
                    detail,
                )
            })
            .collect(),
    )
}

struct Flow<'s, 'a> {
    semantic: &'s Semantic<'a>,
    definitions: HashMap<SymbolId, &'a Expression<'a>>,
    functions: HashMap<SymbolId, NodeId>,
    imports: HashMap<SymbolId, String>,
    setters: HashMap<SymbolId, NodeId>,
    receiver_setters: HashMap<NodeId, NodeId>,
    candidates: HashSet<SymbolId>,
    calls: Vec<(&'a CallExpression<'a>, Option<NodeId>)>,
    budget: Cell<usize>,
    exhausted: Cell<bool>,
    cancel: &'s Cancellation,
}

#[derive(Clone)]
struct Context<'a> {
    function: NodeId,
    environment: Environment<'a>,
    entries: Vec<(NodeId, u32)>,
    depth: usize,
}

fn function(e: &Expression<'_>) -> Option<NodeId> {
    match e.get_inner_expression() {
        Expression::FunctionExpression(f) => Some(f.node_id.get()),
        Expression::ArrowFunctionExpression(f) => Some(f.node_id.get()),
        _ => None,
    }
}

fn argument<'a>(call: &'a CallExpression<'a>, index: usize) -> Option<&'a Expression<'a>> {
    call.arguments.get(index)?.as_expression()
}
