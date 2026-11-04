//! Binding-wide exclusions for state that does not represent a page snapshot.
//! Unknown references keep the entire binding, including all its write sites.

use super::super::JsOptions;
use crate::model::{Analysis, Rule};
use crate::scheduler::Cancellation;
use oxc_ast::{AstKind, ast::*};
use oxc_semantic::Semantic;
use oxc_span::GetSpan;
use oxc_syntax::{node::NodeId, symbol::SymbolId};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

mod generation;
mod module_cache;

pub(crate) fn prune(
    analysis: &mut Analysis,
    semantic: &Semantic<'_>,
    options: &JsOptions,
    cancel: &Cancellation,
    sources: Option<(&sourcemap::DecodedMap, &str)>,
) {
    let candidates: HashSet<u32> = analysis
        .findings
        .iter()
        .filter(|f| f.rule == Rule::ModuleStateWrite)
        .filter_map(|f| {
            f.declaration
                .as_ref()
                .filter(|d| d.is_let)
                .map(|d| d.span.start)
        })
        .collect();
    if candidates.is_empty()
        || semantic.nodes().len() > options.max_steps
        || cancel.check().is_err()
        || analysis.gaps.iter().any(|gap| {
            matches!(
                gap.reason.as_str(),
                "parse-error" | "step-budget" | "time-budget" | "cancelled"
            )
        })
    {
        return;
    }
    let budget = Cell::new(options.max_steps);
    let patterns = Patterns::new(semantic, sources, &budget, cancel);
    let mut excluded = HashSet::new();
    for symbol in semantic.scoping().symbol_ids() {
        let start = semantic.scoping().symbol_span(symbol).start;
        if !candidates.contains(&start) || cancel.check().is_err() {
            continue;
        }
        // A property write is not a scalar sequence or a paired binding reset.
        if analysis.findings.iter().any(|f| {
            f.rule == Rule::ModuleStateWrite
                && f.declaration
                    .as_ref()
                    .is_some_and(|d| d.span.start == start && f.target != d.name)
        }) {
            continue;
        }
        if patterns.sequence(symbol)
            || patterns.generation(symbol)
            || patterns.synchronous_reset(symbol)
            || patterns.lifecycle_reset(symbol)
            || patterns.module_cache(symbol)
        {
            excluded.insert(start);
        }
    }
    analysis.findings.retain(|f| {
        f.rule != Rule::ModuleStateWrite
            || !f
                .declaration
                .as_ref()
                .is_some_and(|d| excluded.contains(&d.span.start))
    });
}

struct Patterns<'s, 'a> {
    semantic: &'s Semantic<'a>,
    definitions: HashMap<SymbolId, &'a Expression<'a>>,
    functions: HashMap<SymbolId, NodeId>,
    returns: HashMap<NodeId, Vec<&'a Expression<'a>>>,
    edges: HashMap<NodeId, HashSet<NodeId>>,
    sources: Option<(&'s sourcemap::DecodedMap, &'s str)>,
    budget: &'s Cell<usize>,
    cancel: &'s Cancellation,
}

impl<'s, 'a> Patterns<'s, 'a> {
    fn new(
        semantic: &'s Semantic<'a>,
        sources: Option<(&'s sourcemap::DecodedMap, &'s str)>,
        budget: &'s Cell<usize>,
        cancel: &'s Cancellation,
    ) -> Self {
        let mut definitions = HashMap::new();
        let mut functions = HashMap::new();
        for (node, item) in semantic.nodes().iter_enumerated() {
            match item.kind() {
                AstKind::VariableDeclarator(d) => {
                    if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&d.id, &d.init) {
                        if let Some(symbol) = id.symbol_id.get() {
                            definitions.insert(symbol, init);
                            if let Some(function) = function_node(init) {
                                functions.insert(symbol, function);
                            }
                        }
                    }
                }
                AstKind::Function(f) if f.is_declaration() => {
                    if let Some(symbol) = f.id.as_ref().and_then(|id| id.symbol_id.get()) {
                        functions.insert(symbol, node);
                    }
                }
                _ => {}
            }
        }
        let mut result = Self {
            semantic,
            definitions,
            functions,
            returns: HashMap::new(),
            edges: HashMap::new(),
            sources,
            budget,
            cancel,
        };
        result.functions.retain(|symbol, _| {
            !semantic
                .scoping()
                .get_resolved_references(*symbol)
                .any(|r| r.is_write())
        });
        for (id, n) in semantic.nodes().iter_enumerated() {
            match n.kind() {
                AstKind::ReturnStatement(r) => {
                    if let (Some(owner), Some(e)) = (result.owner(id), r.argument.as_ref()) {
                        result.returns.entry(owner).or_default().push(e);
                    }
                }
                AstKind::ArrowFunctionExpression(f) => {
                    if let Some(e) = f.body.as_expression() {
                        result.returns.entry(id).or_default().push(e);
                    }
                }
                AstKind::CallExpression(c) => {
                    if let (Some(owner), Some(callee)) =
                        (result.owner(id), result.function(&c.callee))
                    {
                        result.edges.entry(owner).or_default().insert(callee);
                    }
                }
                _ => {}
            }
        }
        result
    }

    fn symbol(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        id.reference_id
            .get()
            .and_then(|id| self.semantic.scoping().get_reference(id).symbol_id())
    }

    fn step(&self) -> bool {
        let Some(remaining) = self.budget.get().checked_sub(1) else {
            return false;
        };
        self.budget.set(remaining);
        remaining % 256 != 0 || self.cancel.check().is_ok()
    }

    fn expression_symbol(&self, e: &Expression<'_>) -> Option<SymbolId> {
        if let Expression::Identifier(id) = e.get_inner_expression() {
            self.symbol(id)
        } else {
            None
        }
    }

    fn owner(&self, node: NodeId) -> Option<NodeId> {
        self.semantic
            .nodes()
            .ancestors_enumerated(node)
            .find(|(_, n)| {
                matches!(
                    n.kind(),
                    AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
                )
            })
            .map(|(id, _)| id)
    }

    fn writes(&self, symbol: SymbolId) -> Option<Vec<(NodeId, &'a AssignmentExpression<'a>)>> {
        self.semantic.scoping().get_resolved_references(symbol).filter(|r| r.is_write())
            .map(|r| {
                let node = self.semantic.nodes().parent_id(r.node_id());
                match self.semantic.nodes().kind(node) {
                    AstKind::AssignmentExpression(a) if matches!(&a.left, AssignmentTarget::AssignmentTargetIdentifier(id) if self.symbol(id) == Some(symbol)) => Some((node, a)),
                    _ => None,
                }
            }).collect()
    }

    fn initial(&self, symbol: SymbolId) -> Option<Literal> {
        match self.definitions.get(&symbol) {
            Some(e) => self.literal(e),
            None => Some(Literal::Undefined),
        }
    }

    fn literal(&self, e: &Expression<'_>) -> Option<Literal> {
        match e.get_inner_expression() {
            Expression::BooleanLiteral(b) => Some(Literal::Bool(b.value)),
            Expression::NullLiteral(_) => Some(Literal::Null),
            Expression::Identifier(id) if id.name == "undefined" && self.symbol(id).is_none() => {
                Some(Literal::Undefined)
            }
            Expression::UnaryExpression(u)
                if u.operator == UnaryOperator::Void
                    && matches!(
                        u.argument.get_inner_expression(),
                        Expression::NumericLiteral(_)
                    ) =>
            {
                Some(Literal::Undefined)
            }
            _ => None,
        }
    }

    fn exported(&self, symbol: SymbolId) -> bool {
        let nodes = self.semantic.nodes();
        nodes
            .ancestor_kinds(self.semantic.scoping().symbol_declaration(symbol))
            .take_while(|n| !matches!(n, AstKind::Program(_)))
            .any(|n| {
                matches!(
                    n,
                    AstKind::ExportNamedDeclaration(_)
                        | AstKind::ExportDeclaration(_)
                        | AstKind::ExportDefaultDeclaration(_)
                )
            })
            || self
                .semantic
                .scoping()
                .get_resolved_references(symbol)
                .any(|r| matches!(nodes.parent_kind(r.node_id()), AstKind::ExportSpecifier(_)))
    }

    fn numeric(&self, e: &Expression<'_>, symbol: SymbolId, depth: usize) -> bool {
        if depth > 12 || !self.step() {
            return false;
        }
        match e.get_inner_expression() {
            Expression::NumericLiteral(_) => true,
            Expression::Identifier(id) => self.symbol(id).is_some_and(|s| {
                s == symbol
                    || (self.semantic.scoping().symbol_flags(s).is_const_variable()
                        && self
                            .definitions
                            .get(&s)
                            .is_some_and(|e| self.numeric(e, symbol, depth + 1)))
            }),
            Expression::BinaryExpression(b) => {
                matches!(
                    b.operator.as_str(),
                    "+" | "-" | "%" | "===" | "!==" | "==" | "!="
                ) && self.numeric(&b.left, symbol, depth + 1)
                    && self.numeric(&b.right, symbol, depth + 1)
            }
            Expression::ConditionalExpression(c) => {
                self.numeric(&c.test, symbol, depth + 1)
                    && self.numeric(&c.consequent, symbol, depth + 1)
                    && self.numeric(&c.alternate, symbol, depth + 1)
            }
            Expression::CallExpression(c) => match c.callee.get_inner_expression() {
                Expression::Identifier(id) if id.name == "Number" && self.symbol(id).is_none() => {
                    c.arguments.len() == 1
                        && c.arguments[0]
                            .as_expression()
                            .is_some_and(|e| self.numeric(e, symbol, depth + 1))
                }
                Expression::StaticMemberExpression(m)
                    if m.property.name == "getUTCMilliseconds" && c.arguments.is_empty() =>
                {
                    matches!(m.object.get_inner_expression(), Expression::NewExpression(n)
                        if n.arguments.is_empty() && matches!(n.callee.get_inner_expression(), Expression::Identifier(id) if id.name == "Date" && self.symbol(id).is_none()))
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn sequence(&self, symbol: SymbolId) -> bool {
        if self.exported(symbol)
            || self.definitions.get(&symbol).is_some_and(|e| {
                !self.numeric(e, symbol, 0) && self.literal(e) != Some(Literal::Undefined)
            })
        {
            return false;
        }
        let nodes = self.semantic.nodes();
        let mut advanced = false;
        let mut formatted = false;
        for reference in self.semantic.scoping().get_resolved_references(symbol) {
            let node = reference.node_id();
            let parent = nodes.parent_kind(node);
            if reference.is_write() {
                match parent {
                    AstKind::UpdateExpression(u) if u.operator.as_str() == "++" => advanced = true,
                    AstKind::AssignmentExpression(a) if self.numeric(&a.right, symbol, 0) => {
                        if !matches!(a.operator.as_str(), "=" | "+=" | "%=") {
                            return false;
                        }
                        advanced |= a.operator.as_str() == "+="
                            || contains_increment(&a.right, symbol, self);
                    }
                    AstKind::AssignmentExpression(a)
                        if a.operator.as_str() == "="
                            && self.literal(&a.right) == Some(Literal::Undefined) => {}
                    _ => return false,
                }
                continue;
            }
            // Reads used only to update this sequence are allowed; reads in
            // control flow or as raw numbers passed elsewhere keep the record.
            if nodes.ancestor_kinds(node).take_while(|n| !matches!(n, AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)))
                .any(|n| matches!(n, AstKind::AssignmentExpression(a) if matches!(&a.left, AssignmentTarget::AssignmentTargetIdentifier(id) if self.symbol(id) == Some(symbol)) && self.numeric(&a.right, symbol, 0))) { continue; }
            let string_use = matches!(parent, AstKind::TemplateLiteral(_))
                || matches!(parent, AstKind::StaticMemberExpression(m) if m.property.name == "toString"
                    && matches!(nodes.parent_kind(m.node_id.get()), AstKind::CallExpression(c) if c.arguments.is_empty()))
                || matches!(parent, AstKind::CallExpression(c) if c.arguments.len() == 1
                    && matches!(c.callee.get_inner_expression(), Expression::Identifier(id) if id.name == "String" && self.symbol(id).is_none()));
            if !string_use {
                return false;
            }
            // Formatting a state flag directly inside a condition or a hook
            // argument does not turn it into an ID generator.
            let output_use = nodes.ancestor_kinds(node).find_map(|kind| match kind {
                AstKind::IdentifierReference(_) | AstKind::StaticMemberExpression(_) | AstKind::TemplateLiteral(_)
                    | AstKind::ParenthesizedExpression(_) => None,
                AstKind::CallExpression(c) if matches!(c.callee.get_inner_expression(), Expression::StaticMemberExpression(m) if m.property.name == "toString")
                    || matches!(c.callee.get_inner_expression(), Expression::Identifier(id) if id.name == "String" && self.symbol(id).is_none()) => None,
                AstKind::BinaryExpression(b) if b.operator.as_str() == "+" => None,
                AstKind::ReturnStatement(_) | AstKind::VariableDeclarator(_) => Some(true),
                _ => Some(false),
            }).unwrap_or(false);
            if !output_use {
                return false;
            }
            formatted = true;
        }
        advanced && formatted
    }

    fn synchronous_reset(&self, symbol: SymbolId) -> bool {
        let Some(initial @ Literal::Bool(_)) = self.initial(symbol) else {
            return false;
        };
        let Some(writes) = self.writes(symbol) else {
            return false;
        };
        let mut covered = HashSet::new();
        for (node, write) in &writes {
            if write.operator.as_str() != "=" || self.literal(&write.right).is_none() {
                return false;
            }
            if self.literal(&write.right) == Some(initial) {
                continue;
            }
            let nodes = self.semantic.nodes();
            let Some(owner) = self.owner(*node) else {
                return false;
            };
            if matches!(nodes.kind(owner), AstKind::Function(f) if f.r#async || f.generator)
                || matches!(nodes.kind(owner), AstKind::ArrowFunctionExpression(f) if f.r#async)
            {
                return false;
            }
            let AstKind::ExpressionStatement(statement) = nodes.parent_kind(*node) else {
                return false;
            };
            let statements = match nodes.parent_kind(statement.node_id.get()) {
                AstKind::BlockStatement(b) => &b.body,
                AstKind::FunctionBody(b) => &b.statements,
                _ => return false,
            };
            let Some(position) = statements.iter().position(|s| s.span() == statement.span) else {
                return false;
            };
            let Some(Statement::TryStatement(next)) = statements.get(position + 1) else {
                return false;
            };
            let Some(finalizer) = &next.finalizer else {
                return false;
            };
            let Some(Statement::ExpressionStatement(reset)) = finalizer.body.first() else {
                return false;
            };
            let Expression::AssignmentExpression(reset) = reset.expression.get_inner_expression()
            else {
                return false;
            };
            if reset.operator.as_str() != "="
                || self.literal(&reset.right) != Some(initial)
                || !matches!(&reset.left, AssignmentTarget::AssignmentTargetIdentifier(id) if self.symbol(id) == Some(symbol))
            {
                return false;
            }
            // Captured reads/writes can outlive this synchronous call.
            if self
                .semantic
                .scoping()
                .get_resolved_references(symbol)
                .any(|r| self.owner(r.node_id()) != Some(owner))
            {
                return false;
            }
            covered.insert(write.span.start);
            covered.insert(reset.span.start);
        }
        !writes.is_empty() && writes.iter().all(|(_, w)| covered.contains(&w.span.start))
    }

    fn function(&self, e: &Expression<'_>) -> Option<NodeId> {
        function_node(e).or_else(|| {
            self.expression_symbol(e)
                .and_then(|s| self.functions.get(&s).copied())
        })
    }

    fn returned(&self, function: NodeId) -> Vec<&'a Expression<'a>> {
        self.returns.get(&function).cloned().unwrap_or_default()
    }

    fn reachable(&self, function: NodeId) -> HashSet<NodeId> {
        if self.is_async(function) {
            return HashSet::new();
        }
        let mut reached = HashSet::from([function]);
        for _ in 0..8 {
            let before = reached.len();
            let next: Vec<_> = reached
                .iter()
                .filter_map(|owner| self.edges.get(owner))
                .flatten()
                .copied()
                .collect();
            reached.extend(next.into_iter().filter(|callee| !self.is_async(*callee)));
            if reached.len() == before {
                break;
            }
        }
        reached
    }

    fn is_async(&self, function: NodeId) -> bool {
        matches!(self.semantic.nodes().kind(function), AstKind::Function(f) if f.r#async || f.generator)
            || matches!(self.semantic.nodes().kind(function), AstKind::ArrowFunctionExpression(f) if f.r#async)
    }

    fn lifecycle_reset(&self, symbol: SymbolId) -> bool {
        let Some(initial) = self.initial(symbol) else {
            return false;
        };
        let Some(writes) = self.writes(symbol) else {
            return false;
        };
        if writes.is_empty() || writes.iter().any(|(_, a)| a.operator.as_str() != "=") {
            return false;
        }
        let mut covered = HashSet::new();
        // A returned disposer belongs to its registration function. Following
        // local helper calls also covers acquisition/release helper pairs.
        for (&setup, returned_values) in &self.returns {
            for returned in returned_values {
                let Some(cleanup) = function_node(returned) else {
                    continue;
                };
                let setup_functions = self.reachable(setup);
                let cleanup_functions = self.reachable(cleanup);
                let resets: Vec<_> = writes
                    .iter()
                    .filter(|(node, a)| {
                        self.literal(&a.right) == Some(initial)
                            && self
                                .owner(*node)
                                .is_some_and(|owner| cleanup_functions.contains(&owner))
                    })
                    .collect();
                if resets.is_empty() {
                    continue;
                }
                for (node, a) in &writes {
                    if self
                        .owner(*node)
                        .is_some_and(|owner| setup_functions.contains(&owner))
                    {
                        covered.insert(a.span.start);
                    }
                }
                for (_, a) in resets {
                    covered.insert(a.span.start);
                }
            }
        }
        writes.iter().all(|(_, a)| covered.contains(&a.span.start))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Literal {
    Undefined,
    Null,
    Bool(bool),
}

fn function_node(e: &Expression<'_>) -> Option<NodeId> {
    match e.get_inner_expression() {
        Expression::FunctionExpression(f) => Some(f.node_id.get()),
        Expression::ArrowFunctionExpression(f) => Some(f.node_id.get()),
        _ => None,
    }
}

fn contains_increment(e: &Expression<'_>, symbol: SymbolId, patterns: &Patterns<'_, '_>) -> bool {
    match e.get_inner_expression() {
        Expression::BinaryExpression(b) => {
            b.operator.as_str() == "+"
                && patterns.expression_symbol(&b.left) == Some(symbol)
                && matches!(b.right.get_inner_expression(), Expression::NumericLiteral(n) if n.value > 0.0)
        }
        Expression::ConditionalExpression(c) => {
            contains_increment(&c.consequent, symbol, patterns)
                || contains_increment(&c.alternate, symbol, patterns)
        }
        _ => false,
    }
}
