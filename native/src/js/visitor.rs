//! AST walk.
//!
//! The visitor drives the rules in source order, maintains the `this`
//! stack, binds `var`/`const` values, follows IIFE arguments into
//! parameters, and records module dependencies.

use super::Analyzer;
use super::values::Value;
use crate::model::{DependencyKind, ScriptMode, Span};
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_span::GetSpan;
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;

fn span_of(s: oxc_span::Span) -> Span {
    Span::new(s.start, s.end)
}

fn is_nullish(expr: &Expression<'_>) -> bool {
    match expr.get_inner_expression() {
        Expression::NullLiteral(_) => true,
        Expression::Identifier(id) => id.name == "undefined",
        _ => false,
    }
}

/// Whether `expr` is a null check in the direction a short-circuit guard
/// needs: for `a || b`, `a` must test *equality* with null/undefined; for
/// `a && b`, *inequality*. `typeof x === "undefined"` counts as equality.
fn is_null_guard(expr: &Expression<'_>, equality: bool) -> bool {
    let Expression::BinaryExpression(b) = expr.get_inner_expression() else {
        return false;
    };
    let operator_matches = if equality {
        matches!(
            b.operator,
            BinaryOperator::Equality | BinaryOperator::StrictEquality
        )
    } else {
        matches!(
            b.operator,
            BinaryOperator::Inequality | BinaryOperator::StrictInequality
        )
    };
    if !operator_matches {
        return false;
    }
    if is_nullish(&b.left) || is_nullish(&b.right) {
        return true;
    }
    let typeof_undefined = |a: &Expression<'_>, other: &Expression<'_>| {
        matches!(a.get_inner_expression(), Expression::UnaryExpression(u) if u.operator == UnaryOperator::Typeof)
            && matches!(other.get_inner_expression(), Expression::StringLiteral(s) if s.value == "undefined")
    };
    typeof_undefined(&b.left, &b.right) || typeof_undefined(&b.right, &b.left)
}

/// Static type name of an initializer for the `detail` field.
fn literal_type(e: &Expression<'_>) -> Option<&'static str> {
    match e.get_inner_expression() {
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => Some("string"),
        Expression::NumericLiteral(_) => Some("number"),
        Expression::BooleanLiteral(_) => Some("boolean"),
        Expression::NullLiteral(_) => Some("null"),
        Expression::ObjectExpression(_) => Some("object"),
        Expression::ArrayExpression(_) => Some("array"),
        Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => {
            Some("function")
        }
        Expression::ClassExpression(_) => Some("class"),
        _ => None,
    }
}

impl<'s, 'c> Analyzer<'s, 'c> {
    /// Binds the parameters of an immediately-invoked function to the
    /// abstract values of its arguments so UMD/bundler wrappers resolve.
    fn bind_iife_params(&mut self, params: &FormalParameters<'_>, args: &[Argument<'_>]) {
        let values: Vec<Value> = args
            .iter()
            .map(|a| {
                a.as_expression()
                    .map(|e| self.eval(e))
                    .unwrap_or(Value::Unknown)
            })
            .collect();
        for ((param, value), arg) in params.items.iter().zip(values).zip(args.iter()) {
            self.bind_pattern(&param.pattern, value);
            // Keep the argument's reference identity, not only its abstract
            // value, so removals inside the IIFE pair with registrations
            // made with the same handler outside it.
            if let BindingPattern::BindingIdentifier(id) = &param.pattern {
                if let (Some(symbol), Some(expr)) = (id.symbol_id.get(), arg.as_expression()) {
                    self.record_alias(symbol, expr);
                }
            }
        }
    }

    /// Binds a destructuring pattern to an abstract value.
    fn bind_pattern(&mut self, pattern: &BindingPattern<'_>, value: Value) {
        match pattern {
            BindingPattern::BindingIdentifier(id) => {
                if let Some(symbol) = id.symbol_id.get() {
                    self.values.insert(symbol, value);
                }
            }
            BindingPattern::ObjectPattern(o) => {
                for prop in &o.properties {
                    if let Some(key) = prop.key.static_name() {
                        let v = self.member(value.clone(), &key);
                        self.bind_pattern(&prop.value, v);
                    }
                }
            }
            BindingPattern::ArrayPattern(a) => {
                for (i, element) in a.elements.iter().enumerate() {
                    if let Some(element) = element {
                        let v = value.property(&i.to_string());
                        self.bind_pattern(element, v);
                    }
                }
            }
            BindingPattern::AssignmentPattern(a) => self.bind_pattern(&a.left, value),
        }
    }

    /// The `this` value inside a non-arrow function. Only the root scope of a
    /// classic script has a known `this`; `call(window)`-style invocations
    /// are handled where the IIFE is visited.
    fn function_this(&self) -> Value {
        Value::Unknown
    }

    fn param_symbols(params: &FormalParameters<'_>) -> Vec<Option<(SymbolId, String)>> {
        params
            .items
            .iter()
            .map(|p| match &p.pattern {
                BindingPattern::BindingIdentifier(id) => {
                    id.symbol_id.get().map(|s| (s, id.name.to_string()))
                }
                BindingPattern::AssignmentPattern(a) => match &a.left {
                    BindingPattern::BindingIdentifier(id) => {
                        id.symbol_id.get().map(|s| (s, id.name.to_string()))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// Whether an expression is a function or arrow literal, with params.
    fn fn_literal_params(expr: &Expression<'_>) -> Option<Vec<Option<(SymbolId, String)>>> {
        match expr.get_inner_expression() {
            Expression::FunctionExpression(f) => Some(Self::param_symbols(&f.params)),
            Expression::ArrowFunctionExpression(f) => Some(Self::param_symbols(&f.params)),
            _ => None,
        }
    }

    /// Tracks reference aliases: `const g = f`, `const h = this.handler`,
    /// and `const t = this` (the last under `this_aliases`).
    fn record_alias(&mut self, symbol: SymbolId, init: &Expression<'_>) {
        match init.get_inner_expression() {
            Expression::ThisExpression(_) => {
                self.this_aliases.insert(symbol);
                self.aliases.remove(&symbol);
            }
            _ => {
                self.this_aliases.remove(&symbol);
                match self.callback_identity(init) {
                    Some(identity) => {
                        self.aliases.insert(symbol, identity);
                    }
                    None => {
                        self.aliases.remove(&symbol);
                    }
                }
            }
        }
    }

    /// Records function literals that form a return value (directly, as the
    /// last operand of a comma sequence, or inside a returned object/array),
    /// so removals inside them can clean up registrations made by the
    /// returning function.
    fn mark_returned(&mut self, expr: &Expression<'_>) {
        match expr.get_inner_expression() {
            Expression::FunctionExpression(f) => {
                self.returned_fns.insert(f.span().start);
            }
            Expression::ArrowFunctionExpression(f) => {
                self.returned_fns.insert(f.span().start);
            }
            Expression::SequenceExpression(s) => {
                if let Some(last) = s.expressions.last() {
                    self.mark_returned(last);
                }
            }
            Expression::ObjectExpression(o) => {
                for prop in &o.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = prop {
                        self.mark_returned(&p.value);
                    }
                }
            }
            Expression::ArrayExpression(a) => {
                for element in &a.elements {
                    if let Some(e) = element.as_expression() {
                        self.mark_returned(e);
                    }
                }
            }
            Expression::ConditionalExpression(c) => {
                self.mark_returned(&c.consequent);
                self.mark_returned(&c.alternate);
            }
            Expression::LogicalExpression(l) => {
                self.mark_returned(&l.left);
                self.mark_returned(&l.right);
            }
            _ => {}
        }
    }
}

impl<'a, 's, 'c> Visit<'a> for Analyzer<'s, 'c> {
    fn visit_expression(&mut self, expr: &Expression<'a>) {
        if !self.step(span_of(expr.span())) {
            return;
        }
        walk::walk_expression(self, expr);
    }

    fn visit_statement(&mut self, stmt: &Statement<'a>) {
        if !self.step(span_of(stmt.span())) {
            return;
        }
        walk::walk_statement(self, stmt);
    }

    // ----- declarations ---------------------------------------------------

    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        let binding = match &decl.id {
            BindingPattern::BindingIdentifier(id) => id.symbol_id.get(),
            _ => None,
        };
        if let Some(init) = &decl.init {
            if let (Some(symbol), Some(params)) = (binding, Self::fn_literal_params(init)) {
                self.fn_params.insert(symbol, params);
                self.pending_owner.push(Some(symbol));
                self.visit_expression(init);
                self.pending_owner.pop();
            } else {
                self.visit_expression(init);
            }
            let value = self.eval(init);
            self.bind_pattern(&decl.id, value);
            if let Some(symbol) = binding {
                self.record_alias(symbol, init);
            }
        }
        if let BindingPattern::BindingIdentifier(id) = &decl.id {
            self.rule_declaration(id, decl.init.as_ref().and_then(literal_type));
        }
    }

    fn visit_return_statement(&mut self, stmt: &ReturnStatement<'a>) {
        if let Some(argument) = &stmt.argument {
            self.mark_returned(argument);
        }
        walk::walk_return_statement(self, stmt);
    }

    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        let returned = self.returned_fns.contains(&func.span().start);
        if returned {
            self.cleanup_blocks.push(self.block);
        }
        let owner = func
            .id
            .as_ref()
            .and_then(|id| id.symbol_id.get())
            .or_else(|| self.pending_owner.last().copied().flatten());
        if let Some(symbol) = owner {
            self.fn_params
                .insert(symbol, Self::param_symbols(&func.params));
        }
        if let Some(id) = &func.id {
            if func.is_declaration() {
                self.rule_declaration(id, Some("function"));
            }
        }
        let this = self.function_this();
        let values = self.values.clone();
        let aliases = self.aliases.clone();
        let this_aliases = self.this_aliases.clone();
        // Conditionality is intra-procedural: a function defined under a
        // branch still executes its own body top to bottom when called.
        let depth = std::mem::replace(&mut self.conditional_depth, 0);
        self.this_stack.push(this);
        self.fn_stack.push(owner);
        walk::walk_function(self, func, flags);
        self.fn_stack.pop();
        self.this_stack.pop();
        self.conditional_depth = depth;
        self.values = values;
        self.aliases = aliases;
        self.this_aliases = this_aliases;
        if returned {
            self.cleanup_blocks.pop();
        }
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        // Classes are block-scoped; they never create window properties.
        walk::walk_class(self, class);
    }

    fn visit_arrow_function_expression(&mut self, func: &ArrowFunctionExpression<'a>) {
        // Arrow functions inherit `this`; no stack change.
        let returned = self.returned_fns.contains(&func.span.start);
        if returned {
            self.cleanup_blocks.push(self.block);
        }
        let owner = self.pending_owner.last().copied().flatten();
        if let Some(symbol) = owner {
            self.fn_params
                .insert(symbol, Self::param_symbols(&func.params));
        }
        let values = self.values.clone();
        let aliases = self.aliases.clone();
        let this_aliases = self.this_aliases.clone();
        let depth = std::mem::replace(&mut self.conditional_depth, 0);
        let block = std::mem::replace(&mut self.block, func.span.start + 1);
        self.fn_stack.push(owner);
        walk::walk_arrow_function_expression(self, func);
        self.fn_stack.pop();
        self.block = block;
        self.conditional_depth = depth;
        self.values = values;
        self.aliases = aliases;
        self.this_aliases = this_aliases;
        if returned {
            self.cleanup_blocks.pop();
        }
    }

    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        let block = std::mem::replace(&mut self.block, body.span.start + 1);
        let strict = self.strict;
        self.strict |= body.directives.iter().any(|d| d.directive == "use strict");
        walk::walk_function_body(self, body);
        self.strict = strict;
        self.block = block;
    }

    fn visit_block_statement(&mut self, block: &BlockStatement<'a>) {
        let prior = std::mem::replace(&mut self.block, block.span.start + 1);
        walk::walk_block_statement(self, block);
        self.block = prior;
    }

    fn visit_if_statement(&mut self, stmt: &IfStatement<'a>) {
        self.visit_expression(&stmt.test);
        let before = self.values.clone();
        self.conditional_depth += 1;
        self.visit_statement(&stmt.consequent);
        let consequent = std::mem::replace(&mut self.values, before);
        if let Some(alternate) = &stmt.alternate {
            self.visit_statement(alternate);
        }
        for (symbol, value) in consequent {
            let alternate = self.values.get(&symbol).cloned().unwrap_or(Value::Unknown);
            self.values
                .insert(symbol, value.join(alternate, self.options.max_candidates));
        }
        self.conditional_depth -= 1;
    }

    fn visit_conditional_expression(&mut self, expr: &ConditionalExpression<'a>) {
        self.conditional_depth += 1;
        walk::walk_conditional_expression(self, expr);
        self.conditional_depth -= 1;
    }

    fn visit_logical_expression(&mut self, expr: &LogicalExpression<'a>) {
        // The left operand always runs. The right operand is business-logic
        // conditional except for null guards (`document == null || …`,
        // `x != null && …`), which are the desugared form of `?.` and do
        // not make the right side a conditional cleanup.
        self.visit_expression(&expr.left);
        let guarded = match expr.operator {
            LogicalOperator::Or => is_null_guard(&expr.left, true),
            LogicalOperator::And => is_null_guard(&expr.left, false),
            LogicalOperator::Coalesce => false,
        };
        if !guarded {
            self.conditional_depth += 1;
        }
        self.visit_expression(&expr.right);
        if !guarded {
            self.conditional_depth -= 1;
        }
    }

    fn visit_switch_statement(&mut self, stmt: &SwitchStatement<'a>) {
        self.conditional_depth += 1;
        walk::walk_switch_statement(self, stmt);
        self.conditional_depth -= 1;
    }

    fn visit_while_statement(&mut self, stmt: &WhileStatement<'a>) {
        self.conditional_depth += 1;
        walk::walk_while_statement(self, stmt);
        self.conditional_depth -= 1;
    }

    fn visit_for_statement(&mut self, stmt: &ForStatement<'a>) {
        self.conditional_depth += 1;
        walk::walk_for_statement(self, stmt);
        self.conditional_depth -= 1;
    }

    fn visit_for_of_statement(&mut self, stmt: &ForOfStatement<'a>) {
        self.conditional_depth += 1;
        walk::walk_for_of_statement(self, stmt);
        self.conditional_depth -= 1;
    }

    fn visit_for_in_statement(&mut self, stmt: &ForInStatement<'a>) {
        self.conditional_depth += 1;
        walk::walk_for_in_statement(self, stmt);
        self.conditional_depth -= 1;
    }

    // ----- effects --------------------------------------------------------

    fn visit_assignment_expression(&mut self, a: &AssignmentExpression<'a>) {
        let assigned = match &a.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => self.symbol_of(id),
            _ => None,
        };
        if let (Some(symbol), Some(params)) = (assigned, Self::fn_literal_params(&a.right)) {
            self.fn_params.insert(symbol, params);
            self.pending_owner.push(Some(symbol));
            self.visit_expression(&a.right);
            self.pending_owner.pop();
        } else {
            self.visit_expression(&a.right);
        }
        if a.operator.as_str() == "=" {
            if let Some(symbol) = assigned {
                self.record_alias(symbol, &a.right);
            }
        }
        self.rule_assignment(a);
        self.visit_assignment_target(&a.left);
    }

    fn visit_update_expression(&mut self, u: &UpdateExpression<'a>) {
        self.rule_update(u);
        walk::walk_update_expression(self, u);
    }

    fn visit_unary_expression(&mut self, u: &UnaryExpression<'a>) {
        if u.operator == UnaryOperator::Delete {
            self.rule_delete(u);
        }
        walk::walk_unary_expression(self, u);
    }

    fn visit_call_expression(&mut self, c: &CallExpression<'a>) {
        // IIFE: `(function (root, factory) { … })(window, …)` and the
        // `.call(this)` form used by CoffeeScript/TypeScript output.
        let callee = c.callee.get_inner_expression();
        match callee {
            Expression::FunctionExpression(f) => {
                for arg in &c.arguments {
                    self.visit_argument(arg);
                }
                self.bind_iife_params(&f.params, &c.arguments);
                // A plain IIFE call has `this === undefined` in strict mode
                // and `window` in sloppy mode.
                let strict = self.strict
                    || f.body
                        .as_ref()
                        .is_some_and(|b| b.directives.iter().any(|d| d.directive == "use strict"));
                let this = if strict {
                    Value::Undefined
                } else {
                    self.this_stack[0].clone()
                };
                self.this_stack.push(this);
                if let Some(body) = &f.body {
                    self.visit_function_body(body);
                }
                self.this_stack.pop();
                return;
            }
            Expression::ArrowFunctionExpression(f) => {
                for arg in &c.arguments {
                    self.visit_argument(arg);
                }
                self.bind_iife_params(&f.params, &c.arguments);
                match &f.body {
                    ArrowFunctionBody::FunctionBody(body) => self.visit_function_body(body),
                    other => {
                        if let Some(e) = other.as_expression() {
                            self.visit_expression(e);
                        }
                    }
                }
                return;
            }
            Expression::StaticMemberExpression(m)
                if matches!(m.property.name.as_str(), "call" | "apply") =>
            {
                if let Expression::FunctionExpression(f) = m.object.get_inner_expression() {
                    let this = c
                        .arguments
                        .first()
                        .and_then(|a| a.as_expression())
                        .map(|e| self.eval(e))
                        .unwrap_or(Value::Unknown);
                    for arg in &c.arguments {
                        self.visit_argument(arg);
                    }
                    if m.property.name == "call" {
                        self.bind_iife_params(&f.params, &c.arguments[1.min(c.arguments.len())..]);
                    }
                    self.this_stack.push(this);
                    if let Some(body) = &f.body {
                        self.visit_function_body(body);
                    }
                    self.this_stack.pop();
                    return;
                }
            }
            _ => {}
        }
        self.rule_call(c);
        walk::walk_call_expression(self, c);
    }

    fn visit_new_expression(&mut self, n: &NewExpression<'a>) {
        self.rule_new(n);
        walk::walk_new_expression(self, n);
    }

    fn visit_with_statement(&mut self, w: &WithStatement<'a>) {
        self.gap("with-statement", span_of(w.span), None);
        walk::walk_with_statement(self, w);
    }

    // ----- modules --------------------------------------------------------

    fn visit_import_declaration(&mut self, i: &ImportDeclaration<'a>) {
        let value = Value::Strings(vec![i.source.value.to_string()]);
        self.dependency(
            DependencyKind::Import,
            span_of(i.span),
            &value,
            Some(ScriptMode::Module),
        );
    }

    fn visit_export_from_declaration(&mut self, e: &ExportFromDeclaration<'a>) {
        let value = Value::Strings(vec![e.source.value.to_string()]);
        self.dependency(
            DependencyKind::Import,
            span_of(e.span),
            &value,
            Some(ScriptMode::Module),
        );
    }

    fn visit_export_all_declaration(&mut self, e: &ExportAllDeclaration<'a>) {
        let value = Value::Strings(vec![e.source.value.to_string()]);
        self.dependency(
            DependencyKind::Import,
            span_of(e.span),
            &value,
            Some(ScriptMode::Module),
        );
    }

    fn visit_import_expression(&mut self, i: &ImportExpression<'a>) {
        let value = self.eval(&i.source);
        self.dependency(
            DependencyKind::DynamicImport,
            span_of(i.span),
            &value,
            Some(ScriptMode::Module),
        );
        walk::walk_import_expression(self, i);
    }
}
