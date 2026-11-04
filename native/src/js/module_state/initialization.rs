//! Initialization-only call chains over the resolved scan graph. A function is
//! eligible only if every reference is a direct call from module evaluation or
//! another eligible function. Unknown uses and public namespaces stop inference.

use crate::lines::LineIndex;
use crate::model::{DependencyKind, Rule, ScriptMode};
use crate::scheduler::{Cancellation, Resource};
use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::*};
use oxc_parser::Parser;
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::{node::NodeId, symbol::SymbolId};
use std::collections::{HashMap, HashSet, VecDeque};

type FunctionKey = (usize, usize);

#[derive(Clone, Copy)]
enum Caller {
    Root,
    Function(usize),
    Unknown,
}

#[derive(Default)]
struct Function {
    callers: Vec<(usize, Caller)>,
    blocked: bool,
}

#[derive(Clone)]
enum Export {
    Local(usize),
    Remote(usize, String),
}

struct ImportUse {
    target: usize,
    name: String,
    callers: Vec<Caller>,
    escaped: bool,
}

#[derive(Default)]
struct Summary {
    functions: Vec<Function>,
    exports: HashMap<String, Export>,
    imports: Vec<ImportUse>,
    opaque_targets: HashSet<usize>,
    writes: HashMap<u32, Vec<Caller>>,
}

pub(crate) fn prune(resources: &mut [Resource], max_steps: usize, cancel: &Cancellation) {
    if !resources.iter().any(|r| {
        r.analysis
            .as_ref()
            .is_some_and(|a| a.findings.iter().any(|f| f.rule == Rule::ModuleStateWrite))
    }) {
        return;
    }
    let mut summaries = Vec::with_capacity(resources.len());
    for (index, resource) in resources.iter().enumerate() {
        if cancel.check().is_err() {
            return;
        }
        summaries.push(summarize(index, resource, max_steps, cancel));
    }
    let mut opaque = HashSet::new();
    let mut uses = Vec::new();
    for (index, resource) in resources.iter().enumerate() {
        if cancel.check().is_err() {
            return;
        }
        if resource.is_entry {
            opaque.insert(index);
        }
        if let Some(summary) = &summaries[index] {
            opaque.extend(&summary.opaque_targets);
            for import in &summary.imports {
                if let Some(target) = resolve(&summaries, import.target, &import.name) {
                    uses.push((target, index, import.callers.clone(), import.escaped));
                }
            }
        }
        // A dynamic namespace or an importer we could not analyze may expose
        // any export. Named re-exports are resolved through the same graph.
        for edge in &resource.edges {
            let Some(target) = edge.to else {
                continue;
            };
            let kind = resource
                .analysis
                .as_ref()
                .and_then(|a| a.dependencies.get(edge.dependency))
                .map(|d| d.kind);
            if summaries[index].is_none() || kind == Some(DependencyKind::DynamicImport) {
                opaque.insert(target);
            }
        }
    }
    let mut blocked = HashSet::new();
    for index in opaque {
        if let Some(summary) = &summaries[index] {
            for name in summary.exports.keys() {
                if let Some(key) = resolve(&summaries, index, name) {
                    blocked.insert(key);
                }
            }
        }
    }
    for (target, caller_resource, callers, escaped) in uses {
        if let Some(summary) = &mut summaries[target.0] {
            let function = &mut summary.functions[target.1];
            function.blocked |= escaped;
            function
                .callers
                .extend(callers.into_iter().map(|c| (caller_resource, c)));
        }
    }
    let mut pending = HashMap::new();
    let mut dependents: HashMap<FunctionKey, Vec<FunctionKey>> = HashMap::new();
    let mut ready = VecDeque::new();
    for (resource, summary) in summaries.iter().enumerate() {
        let Some(summary) = summary else {
            continue;
        };
        for (index, function) in summary.functions.iter().enumerate() {
            let key = (resource, index);
            if function.blocked
                || blocked.contains(&key)
                || function.callers.is_empty()
                || function
                    .callers
                    .iter()
                    .any(|(_, c)| matches!(c, Caller::Unknown))
            {
                continue;
            }
            let callers: HashSet<_> = function
                .callers
                .iter()
                .filter_map(|(r, c)| {
                    if let Caller::Function(f) = c {
                        Some((*r, *f))
                    } else {
                        None
                    }
                })
                .collect();
            pending.insert(key, callers.len());
            if callers.is_empty() {
                ready.push_back(key);
            }
            for caller in callers {
                dependents.entry(caller).or_default().push(key);
            }
        }
    }
    let mut safe = HashSet::new();
    while let Some(key) = ready.pop_front() {
        if cancel.check().is_err() {
            return;
        }
        safe.insert(key);
        for dependent in dependents.get(&key).into_iter().flatten() {
            if let Some(count) = pending.get_mut(dependent) {
                *count -= 1;
                if *count == 0 {
                    ready.push_back(*dependent);
                }
            }
        }
    }
    if cancel.check().is_err() {
        return;
    }
    for (index, resource) in resources.iter_mut().enumerate() {
        let (Some(summary), Some(analysis)) = (&summaries[index], &mut resource.analysis) else {
            continue;
        };
        analysis.findings.retain(|f| {
            if f.rule != Rule::ModuleStateWrite {
                return true;
            }
            !f.declaration
                .as_ref()
                .and_then(|d| summary.writes.get(&d.span.start))
                .is_some_and(|writes| {
                    !writes.is_empty()
                        && writes.iter().all(|c| match c {
                            Caller::Root => true,
                            Caller::Function(f) => safe.contains(&(index, *f)),
                            Caller::Unknown => false,
                        })
                })
        });
    }
}

fn resolve(summaries: &[Option<Summary>], mut resource: usize, name: &str) -> Option<FunctionKey> {
    let mut name = name.to_string();
    let mut seen = HashSet::new();
    while seen.insert((resource, name.clone())) {
        match summaries.get(resource)?.as_ref()?.exports.get(&name)? {
            Export::Local(function) => return Some((resource, *function)),
            Export::Remote(target, imported) => {
                resource = *target;
                name = imported.clone();
            }
        }
    }
    None
}

fn symbol(semantic: &Semantic<'_>, id: &IdentifierReference<'_>) -> Option<SymbolId> {
    id.reference_id
        .get()
        .and_then(|r| semantic.scoping().get_reference(r).symbol_id())
}

fn is_callee(expression: &Expression<'_>, reference: NodeId) -> bool {
    matches!(expression.get_inner_expression(), Expression::Identifier(id) if id.node_id.get() == reference)
}

fn function_node(expression: &Expression<'_>) -> Option<NodeId> {
    match expression.get_inner_expression() {
        Expression::FunctionExpression(f) => Some(f.node_id.get()),
        Expression::ArrowFunctionExpression(f) => Some(f.node_id.get()),
        _ => None,
    }
}

fn owner(semantic: &Semantic<'_>, node: NodeId, functions: &HashMap<NodeId, usize>) -> Caller {
    for (id, item) in semantic.nodes().ancestors_enumerated(node) {
        match item.kind() {
            AstKind::Function(_) | AstKind::ArrowFunctionExpression(_) => {
                return functions
                    .get(&id)
                    .copied()
                    .map_or(Caller::Unknown, Caller::Function);
            }
            AstKind::PropertyDefinition(p) if !p.r#static => return Caller::Unknown,
            _ => {}
        }
    }
    Caller::Root
}

fn summarize(
    index: usize,
    resource: &Resource,
    max_steps: usize,
    cancel: &Cancellation,
) -> Option<Summary> {
    if resource.mode != Some(ScriptMode::Module) {
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
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();
    if parsed.fatal_error || parsed.diagnostics.has_errors() {
        return None;
    }
    let built = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(false)
        .build(&parsed.program);
    let semantic = &built.semantic;
    let nodes = semantic.nodes();
    if nodes.len() > max_steps || cancel.check().is_err() {
        return None;
    }
    // Direct eval can read local bindings without lexical references. Function
    // constructors execute in global scope and cannot access ESM-local setters.
    let direct_eval = nodes.iter_enumerated().any(|(_, node)| {
        matches!(node.kind(),
        AstKind::CallExpression(c) if matches!(c.callee.get_inner_expression(),
            Expression::Identifier(id) if id.name == "eval" && symbol(semantic, id).is_none()))
    });
    let mut summary = Summary::default();
    let mut functions = HashMap::new();
    let mut bindings = HashMap::new();
    let mut definitions = HashMap::new();
    let mut duplicates = HashSet::new();
    for (id, node) in nodes.iter_enumerated() {
        let blocked = match node.kind() {
            AstKind::Function(f) => f.r#async || f.generator,
            AstKind::ArrowFunctionExpression(f) => f.r#async,
            _ => continue,
        };
        functions.insert(id, summary.functions.len());
        summary.functions.push(Function {
            blocked: blocked || direct_eval,
            ..Function::default()
        });
    }
    for (id, node) in nodes.iter_enumerated() {
        let definition = match node.kind() {
            AstKind::Function(f) if f.is_declaration() => {
                f.id.as_ref()
                    .and_then(|i| i.symbol_id.get())
                    .map(|s| (s, id, None))
            }
            AstKind::VariableDeclarator(d) => match (&d.id, &d.init) {
                (BindingPattern::BindingIdentifier(i), Some(e)) => i
                    .symbol_id
                    .get()
                    .zip(function_node(e))
                    .map(|(s, f)| (s, f, None)),
                _ => None,
            },
            AstKind::AssignmentExpression(a) if a.operator.as_str() == "=" => match &a.left {
                AssignmentTarget::AssignmentTargetIdentifier(i) => symbol(semantic, i)
                    .zip(function_node(&a.right))
                    .map(|(s, f)| (s, f, Some(id))),
                _ => None,
            },
            _ => None,
        };
        if let Some((binding, function, assignment)) = definition {
            if bindings.insert(binding, functions[&function]).is_some() {
                duplicates.insert(binding);
            }
            definitions.insert(binding, assignment);
        }
    }
    let targets: HashMap<u32, usize> = resource
        .edges
        .iter()
        .filter_map(|edge| {
            let dependency = analysis.dependencies.get(edge.dependency)?;
            (dependency.kind == DependencyKind::Import).then_some((dependency.span.start, edge.to?))
        })
        .collect();
    let mut imported = HashMap::new();
    for (_, node) in nodes.iter_enumerated() {
        match node.kind() {
            AstKind::ImportDeclaration(i) => {
                let Some(target) = targets.get(&i.span.start).copied() else {
                    continue;
                };
                for specifier in i.specifiers.iter().flatten() {
                    let (binding, name) = match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => {
                            (s.local.symbol_id.get(), s.imported.name().to_string())
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                            (s.local.symbol_id.get(), "default".into())
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
                            summary.opaque_targets.insert(target);
                            continue;
                        }
                    };
                    if let Some(binding) = binding {
                        imported.insert(binding, (target, name));
                    }
                }
            }
            AstKind::ExportFromDeclaration(e) => {
                if let Some(target) = targets.get(&e.span.start) {
                    for s in &e.specifiers {
                        summary.exports.insert(
                            s.exported.name().to_string(),
                            Export::Remote(*target, s.local.name().to_string()),
                        );
                    }
                }
            }
            AstKind::ExportAllDeclaration(e) => {
                if let Some(target) = targets.get(&e.span.start) {
                    summary.opaque_targets.insert(*target);
                }
            }
            _ => {}
        }
    }
    let export_for = |s| {
        bindings.get(&s).copied().map(Export::Local).or_else(|| {
            imported
                .get(&s)
                .map(|(target, name)| Export::Remote(*target, name.clone()))
        })
    };
    for (_, node) in nodes.iter_enumerated() {
        match node.kind() {
            AstKind::ExportNamedDeclaration(e) => {
                for s in &e.specifiers {
                    if let ModuleExportName::IdentifierReference(local) = &s.local {
                        if let Some(export) = symbol(semantic, local).and_then(export_for) {
                            summary
                                .exports
                                .insert(s.exported.name().to_string(), export);
                        }
                    }
                }
            }
            AstKind::ExportDefaultDeclaration(e) => {
                let export = match &e.declaration {
                    ExportDefaultDeclarationKind::Identifier(i) => {
                        symbol(semantic, i).and_then(export_for)
                    }
                    ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                        functions.get(&f.node_id.get()).copied().map(Export::Local)
                    }
                    _ => None,
                };
                if let Some(export) = export {
                    summary.exports.insert("default".into(), export);
                }
            }
            _ => {}
        }
    }
    for (&binding, &function) in &bindings {
        let declaration = semantic.scoping().symbol_declaration(binding);
        if semantic.scoping().symbol_scope_id(binding) == semantic.scoping().root_scope_id()
            && nodes
                .ancestor_kinds(declaration)
                .take_while(|n| !matches!(n, AstKind::Program(_)))
                .any(|n| matches!(n, AstKind::ExportDeclaration(_)))
        {
            summary.exports.insert(
                semantic.scoping().symbol_name(binding).to_string(),
                Export::Local(function),
            );
        }
    }
    let mut mapped = MappedRoots::new(source, resource.source_map.as_ref(), max_steps, cancel);
    let call_owner = |id, mapped: &mut MappedRoots<'_>| {
        let caller = owner(semantic, id, &functions);
        // Only anonymous root-level wrapper callbacks can recover source-level
        // module evaluation. Named/runtime callbacks retain their call graph.
        if matches!(caller, Caller::Function(_))
            && root_wrapper(semantic, id, &functions)
            && mapped.is_root(nodes.kind(id).span().start)
        {
            Caller::Root
        } else {
            caller
        }
    };
    for (&binding, &function) in &bindings {
        let mut callers = Vec::new();
        let mut escaped = duplicates.contains(&binding);
        for reference in semantic.scoping().get_resolved_references(binding) {
            let parent = nodes.parent_id(reference.node_id());
            if reference.is_write() && definitions.get(&binding) == Some(&Some(parent)) {
                continue;
            }
            match nodes.kind(parent) {
                AstKind::CallExpression(c)
                    if is_callee(&c.callee, reference.node_id()) && !reference.is_write() =>
                {
                    callers.push((index, call_owner(parent, &mut mapped)))
                }
                AstKind::ExportSpecifier(_) | AstKind::ExportDefaultDeclaration(_)
                    if !reference.is_write() => {}
                _ => escaped = true,
            }
        }
        summary.functions[function].callers = callers;
        summary.functions[function].blocked |= escaped;
    }
    for (binding, (target, name)) in imported {
        let mut callers = Vec::new();
        let mut escaped = direct_eval;
        for reference in semantic.scoping().get_resolved_references(binding) {
            let parent = nodes.parent_id(reference.node_id());
            match nodes.kind(parent) {
                AstKind::CallExpression(c)
                    if is_callee(&c.callee, reference.node_id()) && !reference.is_write() =>
                {
                    callers.push(call_owner(parent, &mut mapped))
                }
                AstKind::ExportSpecifier(_) | AstKind::ExportDefaultDeclaration(_)
                    if !reference.is_write() => {}
                _ => escaped = true,
            }
        }
        summary.imports.push(ImportUse {
            target,
            name,
            callers,
            escaped,
        });
    }
    let bound: HashSet<_> = bindings.values().copied().collect();
    for (&node, &function) in &functions {
        if bound.contains(&function) {
            continue;
        }
        let parent = nodes.parent_id(node);
        if let AstKind::CallExpression(c) = nodes.kind(parent) {
            if function_node(&c.callee) == Some(node) {
                summary.functions[function]
                    .callers
                    .push((index, call_owner(parent, &mut mapped)));
            }
        }
    }
    let mut write_owners = HashMap::new();
    for (id, node) in nodes.iter_enumerated() {
        if matches!(
            node.kind(),
            AstKind::AssignmentExpression(_)
                | AstKind::UpdateExpression(_)
                | AstKind::UnaryExpression(_)
        ) {
            let span = node.kind().span();
            write_owners.insert((span.start, span.end), owner(semantic, id, &functions));
        }
    }
    for finding in &analysis.findings {
        if finding.rule != Rule::ModuleStateWrite {
            continue;
        }
        if let Some(declaration) = &finding.declaration {
            summary
                .writes
                .entry(declaration.span.start)
                .or_default()
                .push(
                    write_owners
                        .get(&(finding.span.start, finding.span.end))
                        .copied()
                        .unwrap_or(Caller::Unknown),
                );
        }
    }
    if cancel.check().is_err() {
        None
    } else {
        Some(summary)
    }
}

fn root_wrapper(semantic: &Semantic<'_>, call: NodeId, functions: &HashMap<NodeId, usize>) -> bool {
    let nodes = semantic.nodes();
    let Some((node, item)) = nodes
        .ancestors_enumerated(call)
        .find(|(id, _)| functions.contains_key(id))
    else {
        return false;
    };
    if matches!(item.kind(), AstKind::Function(f) if f.id.is_some() || f.r#async || f.generator)
        || matches!(item.kind(), AstKind::ArrowFunctionExpression(f) if f.r#async)
    {
        return false;
    }
    let mut parent = nodes.parent_id(node);
    while matches!(nodes.kind(parent), AstKind::ParenthesizedExpression(_)) {
        parent = nodes.parent_id(parent);
    }
    matches!(nodes.kind(parent), AstKind::CallExpression(_))
        && matches!(owner(semantic, parent, functions), Caller::Root)
}

type Position = (u32, u32);
type Ranges = Vec<(Position, Position)>;

struct MappedRoots<'a> {
    lines: LineIndex<'a>,
    map: Option<&'a sourcemap::DecodedMap>,
    originals: HashMap<(String, usize), Ranges>,
    remaining: usize,
    cancel: &'a Cancellation,
}

impl<'a> MappedRoots<'a> {
    fn new(
        source: &'a str,
        map: Option<&'a sourcemap::DecodedMap>,
        max_steps: usize,
        cancel: &'a Cancellation,
    ) -> Self {
        Self {
            lines: LineIndex::new(source),
            map,
            originals: HashMap::new(),
            remaining: max_steps,
            cancel,
        }
    }

    fn is_root(&mut self, start: u32) -> bool {
        if self.cancel.check().is_err() || self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        let (line, column) = self.lines.position(start);
        let Some(token) = self.map.and_then(|m| m.lookup_token(line - 1, column)) else {
            return false;
        };
        let (Some(name), Some(view)) = (token.get_source(), token.get_source_view()) else {
            return false;
        };
        let key = (name.to_string(), view.source().as_ptr() as usize);
        if !self.originals.contains_key(&key) {
            let allocator = Allocator::default();
            let source_type = SourceType::from_path(name)
                .unwrap_or_else(|_| SourceType::mjs().with_jsx(true))
                .with_module(true);
            let parsed = Parser::new(&allocator, view.source(), source_type).parse();
            let mut ranges = Vec::new();
            if !parsed.fatal_error && !parsed.diagnostics.has_errors() {
                let built = SemanticBuilder::new()
                    .with_build_nodes(true)
                    .with_check_syntax_error(false)
                    .build(&parsed.program);
                let count = built.semantic.nodes().len();
                if count <= self.remaining {
                    self.remaining -= count;
                    let lines = LineIndex::new(view.source());
                    for (id, item) in built.semantic.nodes().iter_enumerated() {
                        if let AstKind::CallExpression(c) = item.kind() {
                            if matches!(owner(&built.semantic, id, &HashMap::new()), Caller::Root) {
                                // Restrict mappings to the callee, excluding
                                // nested callbacks inside a root call's args.
                                ranges.push((
                                    lines.position(c.callee.span().start),
                                    lines.position(c.callee.span().end),
                                ));
                            }
                        }
                    }
                }
            }
            self.originals.insert(key.clone(), ranges);
        }
        let point = (token.get_src_line() + 1, token.get_src_col());
        self.originals[&key]
            .iter()
            .any(|(start, end)| *start <= point && point < *end)
    }
}
