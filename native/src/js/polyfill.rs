use super::Analyzer;
use super::values::{Root, Target, Value};
use crate::model::{Polyfill, PolyfillGuard, Span};
use oxc_ast::ast::*;

/// Reaching this branch proves at least one checked native capability is missing.
pub(super) struct Guard {
    checks: Vec<(String, PolyfillGuard)>,
    depth: usize,
    installed: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Flow {
    Continue,
    Return,
    Unknown,
}

/// A statically resolved global/native reference can supply an implementation
/// alias. Unknown computed properties and document/local objects stay opaque.
fn implementation_reference(value: &Value) -> bool {
    let Value::Targets(targets) = value else {
        return false;
    };
    !targets.is_empty()
        && targets.iter().all(|target| {
            !target.path.iter().any(|key| key == "*")
                && match target.root {
                    Root::Builtin(_) | Root::Host(_) => true,
                    Root::Global => !target.path.is_empty(),
                    Root::Document => false,
                }
        })
}

/// Only structured returns count. Loops, breaks, throws and try/finally are
/// deliberately not used to infer a guard for following statements.
fn statement_flow(statement: &Statement<'_>, depth: usize) -> Flow {
    if depth > 64 {
        return Flow::Unknown;
    }
    match statement {
        Statement::ReturnStatement(_) => Flow::Return,
        Statement::BlockStatement(block) => {
            for statement in &block.body {
                let flow = statement_flow(statement, depth + 1);
                if flow != Flow::Continue {
                    return flow;
                }
            }
            Flow::Continue
        }
        Statement::IfStatement(statement) => {
            let consequent = statement_flow(&statement.consequent, depth + 1);
            let alternate = statement
                .alternate
                .as_ref()
                .map(|statement| statement_flow(statement, depth + 1))
                .unwrap_or(Flow::Continue);
            match (consequent, alternate) {
                (Flow::Return, Flow::Return) => Flow::Return,
                (Flow::Unknown, _) | (_, Flow::Unknown) => Flow::Unknown,
                _ => Flow::Continue,
            }
        }
        Statement::ExpressionStatement(_)
        | Statement::VariableDeclaration(_)
        | Statement::FunctionDeclaration(_)
        | Statement::ClassDeclaration(_)
        | Statement::EmptyStatement(_) => Flow::Continue,
        _ => Flow::Unknown,
    }
}

impl Analyzer<'_, '_> {
    /// Match the missing branch of a native API check, without evaluating code.
    pub(super) fn polyfill_guard(&mut self, test: &Expression<'_>, truthy: bool) -> Option<Guard> {
        let uncertain = self.uncertain;
        let guard = self.polyfill_guard_depth(test, truthy, 0);
        self.uncertain = uncertain;
        guard
    }

    fn polyfill_guard_depth(
        &mut self,
        test: &Expression<'_>,
        truthy: bool,
        depth: usize,
    ) -> Option<Guard> {
        if depth > 64 {
            return None;
        }
        let test = test.get_inner_expression();
        if let Expression::UnaryExpression(unary) = test {
            if unary.operator == UnaryOperator::LogicalNot {
                return self.polyfill_guard_depth(&unary.argument, !truthy, depth + 1);
            }
        }
        if let Expression::LogicalExpression(logical) = test {
            if logical.operator == LogicalOperator::Coalesce {
                return None;
            }
            let left = self.polyfill_guard_depth(&logical.left, truthy, depth + 1);
            let right = self.polyfill_guard_depth(&logical.right, truthy, depth + 1);
            // Both operands must hold for && true / || false. For the other
            // branches either operand can decide the result, so both must
            // establish a missing native capability (not a force/config flag).
            let both = (logical.operator == LogicalOperator::And) == truthy;
            return match (left, right) {
                (Some(mut left), Some(right)) => {
                    for check in right.checks {
                        if !left.checks.contains(&check) {
                            left.checks.push(check);
                        }
                    }
                    (left.checks.len() <= self.options.max_candidates).then_some(left)
                }
                (Some(guard), None) | (None, Some(guard)) if both => Some(guard),
                _ => None,
            };
        }
        if let Expression::BinaryExpression(binary) = test {
            if binary.operator == BinaryOperator::In {
                if truthy {
                    return None;
                }
                let keys = self.eval(&binary.left).strings();
                let [key] = keys.as_slice() else {
                    return None;
                };
                let receiver = self.eval(&binary.right);
                let value = self.member(receiver, key);
                return self.native_polyfill_guard(value, PolyfillGuard::Missing);
            }
        }
        let (api, guard) = if let Expression::BinaryExpression(binary) = test {
            let equal = match binary.operator {
                BinaryOperator::Equality | BinaryOperator::StrictEquality => truthy,
                BinaryOperator::Inequality | BinaryOperator::StrictInequality => !truthy,
                _ => return None,
            };
            let pair = match (
                binary.left.get_inner_expression(),
                binary.right.get_inner_expression(),
            ) {
                (Expression::UnaryExpression(unary), value)
                | (value, Expression::UnaryExpression(unary))
                    if unary.operator == UnaryOperator::Typeof =>
                {
                    (unary, value)
                }
                _ => return None,
            };
            let kind = match pair.1 {
                Expression::StringLiteral(value) => value.value.as_str(),
                Expression::TemplateLiteral(value) if value.expressions.is_empty() => {
                    value.quasis.first()?.value.cooked.as_ref()?.as_str()
                }
                _ => return None,
            };
            let guard = match (kind, equal) {
                ("undefined", true) => PolyfillGuard::Missing,
                ("function", false) => PolyfillGuard::NotFunction,
                _ => return None,
            };
            (&pair.0.argument, guard)
        } else if !truthy {
            (test, PolyfillGuard::Missing)
        } else {
            return None;
        };
        let previous = std::mem::replace(&mut self.uncertain, false);
        let value = self.eval(api);
        self.uncertain = previous;
        self.native_polyfill_guard(value, guard)
    }

    fn native_polyfill_guard(&self, value: Value, guard: PolyfillGuard) -> Option<Guard> {
        let [target] = value.targets() else {
            return None;
        };
        if !matches!(target.root, Root::Builtin(_) | Root::Host(_))
            || target.path.contains(&"*".into())
        {
            return None;
        }
        Some(Guard {
            checks: vec![(target.dotted(), guard)],
            depth: self.fn_stack.len(),
            installed: false,
        })
    }

    /// A returning arm excludes that condition from subsequent statements in
    /// the same block. Nested/deferred functions never establish this fact.
    pub(super) fn polyfill_continuation_guard(
        &mut self,
        statement: &Statement<'_>,
    ) -> Option<Guard> {
        let Statement::IfStatement(statement) = statement else {
            return None;
        };
        let consequent = statement_flow(&statement.consequent, 0) == Flow::Return;
        let alternate = statement
            .alternate
            .as_ref()
            .is_some_and(|statement| statement_flow(statement, 0) == Flow::Return);
        if consequent == alternate {
            return None;
        }
        self.polyfill_guard(&statement.test, alternate)
    }

    pub(super) fn is_polyfill_implementation(
        &mut self,
        expression: &Expression<'_>,
        value: &Value,
    ) -> bool {
        let uncertain = self.uncertain;
        let implementation = self.polyfill_implementation_depth(expression, value, 0);
        self.uncertain = uncertain;
        implementation
    }

    fn polyfill_implementation_depth(
        &mut self,
        expression: &Expression<'_>,
        value: &Value,
        depth: usize,
    ) -> bool {
        if depth > 64 {
            return false;
        }
        if matches!(value, Value::Function) || implementation_reference(value) {
            return true;
        }
        // Factories can select vendor APIs or return a locally created closure.
        // Their bodies are not executed; known scalar/object results do not
        // count as implementations of a callable API.
        if !matches!(value, Value::Unknown | Value::Targets(_)) {
            return false;
        }
        let Expression::CallExpression(call) = expression.get_inner_expression() else {
            return false;
        };
        match call.callee.get_inner_expression() {
            Expression::FunctionExpression(function)
                if !function.r#async && !function.generator =>
            {
                function
                    .body
                    .as_ref()
                    .and_then(|body| body.statements.last())
                    .is_some_and(|statement| self.factory_return(statement, depth + 1))
            }
            Expression::ArrowFunctionExpression(function) if !function.r#async => {
                match &function.body {
                    ArrowFunctionBody::FunctionBody(body) => body
                        .statements
                        .last()
                        .is_some_and(|statement| self.factory_return(statement, depth + 1)),
                    body => body
                        .as_expression()
                        .is_some_and(|expression| self.factory_result(expression, depth + 1)),
                }
            }
            _ => false,
        }
    }

    fn factory_result(&mut self, expression: &Expression<'_>, depth: usize) -> bool {
        let value = self.eval(expression);
        (matches!(value, Value::Function) || implementation_reference(&value))
            || self.polyfill_implementation_depth(expression, &value, depth + 1)
    }

    fn factory_return(&mut self, statement: &Statement<'_>, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        match statement {
            Statement::ReturnStatement(statement) => statement
                .argument
                .as_ref()
                .is_some_and(|expression| self.factory_result(expression, depth + 1)),
            Statement::BlockStatement(block) => block
                .body
                .last()
                .is_some_and(|statement| self.factory_return(statement, depth + 1)),
            Statement::IfStatement(statement) => {
                self.factory_return(&statement.consequent, depth + 1)
                    && statement
                        .alternate
                        .as_ref()
                        .is_some_and(|statement| self.factory_return(statement, depth + 1))
            }
            _ => false,
        }
    }

    /// A synchronous factory may install companion APIs before returning the
    /// checked implementation. Limit this provisional bundle to its RHS walk.
    pub(super) fn enter_polyfill_factory(
        &mut self,
        assignment: &AssignmentExpression<'_>,
    ) -> Option<usize> {
        if assignment.operator.as_str() != "=" {
            return None;
        }
        let Expression::CallExpression(call) = assignment.right.get_inner_expression() else {
            return None;
        };
        let synchronous = match call.callee.get_inner_expression() {
            Expression::FunctionExpression(function) => !function.r#async && !function.generator,
            Expression::ArrowFunctionExpression(function) => !function.r#async,
            _ => false,
        };
        if !synchronous {
            return None;
        }
        let uncertain = self.uncertain;
        let value = self.eval(&assignment.right);
        let implementation = self.is_polyfill_implementation(&assignment.right, &value);
        let targets = self.assignment_targets(&assignment.left);
        self.uncertain = uncertain;
        if !implementation {
            return None;
        }
        let index = self.polyfill_guards.iter().rposition(|condition| {
            condition.depth == self.fn_stack.len()
                && !condition.installed
                && targets.iter().any(|target| {
                    condition
                        .checks
                        .iter()
                        .any(|(name, _)| name == &target.dotted())
                })
        })?;
        self.polyfill_guards[index].installed = true;
        Some(index)
    }

    pub(super) fn leave_polyfill_factory(&mut self, index: Option<usize>) {
        if let Some(index) = index {
            self.polyfill_guards[index].installed = false;
        }
    }

    pub(super) fn mark_polyfill(&mut self, target: &Target, span: Span) {
        if !matches!(target.root, Root::Builtin(_) | Root::Host(_))
            || target.path.contains(&"*".into())
        {
            return;
        }
        let target = target.dotted();
        // Installing the checked API establishes a fallback bundle; subsequent
        // host/builtin installations under that same guard belong to it too.
        let Some(condition) = self.polyfill_guards.iter_mut().rev().find(|condition| {
            condition.depth == self.fn_stack.len()
                && (condition.installed || condition.checks.iter().any(|(name, _)| name == &target))
        }) else {
            return;
        };
        if let Some(finding) = self
            .findings
            .iter_mut()
            .rev()
            .find(|finding| finding.target == target && finding.span == span)
        {
            condition.installed = true;
            let (guard_target, guard) = condition
                .checks
                .iter()
                .find(|(name, _)| name == &target)
                .unwrap_or(&condition.checks[0]);
            finding.polyfill = Some(Polyfill {
                guard: *guard,
                guard_target: guard_target.clone(),
            });
        }
    }

    pub(super) fn is_function_descriptor(value: Option<&Value>) -> bool {
        matches!(value, Some(Value::Object(descriptor)) if ["value", "get", "set"].iter()
            .any(|key| matches!(descriptor.get(*key), Some(Value::Function))))
    }
}
