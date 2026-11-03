use super::Analyzer;
use super::values::{Root, Target, Value};
use crate::model::{Polyfill, PolyfillGuard, Span};
use oxc_ast::ast::*;

impl Analyzer<'_, '_> {
    /// Match the missing branch of a native API check, without evaluating code.
    pub(super) fn polyfill_guard(
        &mut self,
        test: &Expression<'_>,
        truthy: bool,
    ) -> Option<(String, PolyfillGuard, usize, bool)> {
        let test = test.get_inner_expression();
        if let Expression::UnaryExpression(unary) = test {
            if unary.operator == UnaryOperator::LogicalNot {
                return self.polyfill_guard(&unary.argument, !truthy);
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
        let [target] = value.targets() else {
            return None;
        };
        if !matches!(target.root, Root::Builtin(_) | Root::Host(_))
            || target.path.contains(&"*".into())
        {
            return None;
        }
        Some((target.dotted(), guard, self.fn_stack.len(), false))
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
        let Some((guard_target, guard, _, installed)) =
            self.polyfill_guards
                .iter_mut()
                .rev()
                .find(|(name, _, depth, installed)| {
                    *depth == self.fn_stack.len() && (*name == target || *installed)
                })
        else {
            return;
        };
        if let Some(finding) = self
            .findings
            .iter_mut()
            .rev()
            .find(|finding| finding.target == target && finding.span == span)
        {
            *installed = true;
            finding.polyfill = Some(Polyfill {
                guard: *guard,
                guard_target: guard_target.clone(),
            });
        }
    }

    pub(super) fn is_function_descriptor(value: Option<&Value>) -> bool {
        matches!(value, Some(Value::Object(descriptor)) if matches!(descriptor.get("value"), Some(Value::Function)))
    }
}
