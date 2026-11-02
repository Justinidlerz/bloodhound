//! Writes to ESM module state from functions that can outlive page instances.
//! Module evaluation and direct IIFEs are initialization, not later writes.

use super::{Analyzer, values::Value};
use crate::model::{Rule, ScriptMode, Span};
use oxc_ast::ast::*;

impl<'s, 'c> Analyzer<'s, 'c> {
    fn module_binding(&self, id: &IdentifierReference<'_>) -> Option<String> {
        if self.options.script_mode != ScriptMode::Module || self.fn_stack.is_empty() {
            return None;
        }
        let symbol = self.symbol_of(id)?;
        (self.scoping.symbol_scope_id(symbol) == self.scoping.root_scope_id()
            && self.scoping.symbol_flags(symbol).is_variable())
        .then(|| self.scoping.symbol_name(symbol).to_string())
    }

    fn module_state_finding(&mut self, target: String, span: Span, operation: &str) {
        self.uncertain = false;
        self.finding(
            Rule::ModuleStateWrite,
            span,
            target,
            operation,
            Some(
                "This function writes ESM module state. The cached module retains it across page remounts; its initializer does not run again. Verify reset in the page lifecycle; this scan does not prove that reset executes."
                    .into(),
            ),
        );
    }

    pub(super) fn module_state_reference(&mut self, id: &IdentifierReference<'_>) {
        if !id
            .reference_id
            .get()
            .is_some_and(|reference| self.scoping.get_reference(reference).is_write())
        {
            return;
        }
        if let Some(target) = self.module_binding(id) {
            let (span, operation) = self.write_context.unwrap_or((id.span.into(), "write"));
            self.module_state_finding(target, span, operation);
        }
    }

    /// Follow a property path to its lexical binding, without treating a
    /// same-named local variable or an imported binding as module-owned state.
    fn module_state_path(&mut self, mut expression: &Expression<'_>) -> Option<String> {
        let mut path: Vec<String> = vec![];
        loop {
            match expression.get_inner_expression() {
                Expression::Identifier(id) => {
                    let mut target = self.module_binding(id)?;
                    for key in path.iter().rev() {
                        target.push('.');
                        target.push_str(key);
                    }
                    return Some(target);
                }
                Expression::StaticMemberExpression(member) => {
                    path.push(member.property.name.to_string());
                    expression = &member.object;
                }
                Expression::ComputedMemberExpression(member) => {
                    let keys = self.eval(&member.expression).strings();
                    path.push(if keys.len() == 1 {
                        keys[0].clone()
                    } else {
                        "*".into()
                    });
                    expression = &member.object;
                }
                _ => return None,
            }
        }
    }

    pub(super) fn module_state_member(
        &mut self,
        member: &MemberExpression<'_>,
        span: Span,
        operation: &str,
    ) {
        if self.options.script_mode != ScriptMode::Module || self.fn_stack.is_empty() {
            return;
        }
        let (object, key) = match member {
            MemberExpression::StaticMemberExpression(member) => {
                (&member.object, member.property.name.to_string())
            }
            MemberExpression::ComputedMemberExpression(member) => {
                let keys = self.eval(&member.expression).strings();
                (
                    &member.object,
                    if keys.len() == 1 {
                        keys[0].clone()
                    } else {
                        "*".into()
                    },
                )
            }
            _ => return,
        };
        // Global/DOM receivers already have their own rules; a module alias
        // such as `const root = window` does not make them module-owned objects.
        if matches!(self.eval(object), Value::Targets(_) | Value::Node(_)) {
            return;
        }
        if let Some(target) = self.module_state_path(object) {
            self.module_state_finding(format!("{target}.{key}"), span, operation);
        }
    }
}
