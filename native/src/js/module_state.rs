//! Writes to ESM module state from functions that can outlive page instances.
//! Module evaluation and direct IIFEs are initialization, not later writes.

use super::{Analyzer, JsOptions, analyze_as, values::Value};
use crate::lines::LineIndex;
use crate::model::{Analysis, BindingDeclaration, OriginalLocation, Rule, ScriptMode, Span};
use crate::scheduler::Cancellation;
use oxc_ast::ast::*;
use oxc_span::SourceType;
use oxc_syntax::symbol::SymbolId;
use std::collections::HashMap;

/// Bundlers can lower declarations to assignments inside lazy module
/// initializers. A mapped assignment is a persistent-state write only when
/// the original module contains a deferred write at that position.
pub(super) fn refine(
    analysis: &mut Analysis,
    source: &str,
    map: Option<&sourcemap::DecodedMap>,
    options: &JsOptions,
    cancel: &Cancellation,
) {
    if !analysis
        .findings
        .iter()
        .any(|finding| finding.rule == Rule::ModuleStateWrite)
    {
        return;
    }
    let generated = LineIndex::new(source);
    let mut originals = HashMap::new();
    analysis.findings.retain_mut(|finding| {
        if finding.rule != Rule::ModuleStateWrite {
            return true;
        }
        let keep_generated = finding
            .declaration
            .as_ref()
            .is_some_and(|declaration| declaration.is_let);
        let Some(map) = map else {
            return keep_generated;
        };
        let (line, column) = generated.position(finding.span.start);
        let Some(token) = map.lookup_token(line - 1, column) else {
            return keep_generated;
        };
        let (Some(name), Some(view)) = (token.get_source(), token.get_source_view()) else {
            return keep_generated;
        };
        // Indexed maps can reuse filenames for different embedded sources.
        let writes = originals
            .entry((name, view.source().as_ptr()))
            .or_insert_with(|| {
                let source_type = SourceType::from_path(name)
                    .unwrap_or_else(|_| SourceType::mjs().with_jsx(true))
                    .with_module(true);
                let original = analyze_as(view.source(), source_type, options, cancel);
                if original.gaps.iter().any(|gap| {
                    matches!(
                        gap.reason.as_str(),
                        "parse-error" | "step-budget" | "time-budget" | "cancelled"
                    )
                }) {
                    return None;
                }
                let index = LineIndex::new(view.source());
                Some(
                    original
                        .findings
                        .iter()
                        .filter(|finding| {
                            matches!(finding.operation.as_str(), "write" | "update" | "delete")
                                && (finding.rule != Rule::ModuleStateWrite
                                    || finding
                                        .declaration
                                        .as_ref()
                                        .is_some_and(|declaration| declaration.is_let))
                        })
                        .map(|finding| {
                            (
                                index.position(finding.span.start),
                                index.position(finding.span.end),
                                finding.clone(),
                                finding.declaration.as_ref().map(|declaration| {
                                    let location = index.location(declaration.span);
                                    OriginalLocation {
                                        source: name.to_string(),
                                        line: location.line,
                                        column: location.column,
                                        end_line: Some(location.end_line),
                                        end_column: Some(location.end_column),
                                        name: Some(declaration.name.clone()),
                                    }
                                }),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            });
        let point = (token.get_src_line() + 1, token.get_src_col());
        let Some(writes) = writes else {
            return keep_generated;
        };
        let member_write = finding
            .declaration
            .as_ref()
            .is_some_and(|declaration| finding.target != declaration.name);
        let Some((_, _, original_finding, original)) =
            writes.iter().find(|(start, end, original, _)| {
                *start <= point
                    && point < *end
                    && (original.rule == Rule::ModuleStateWrite || member_write)
            })
        else {
            return false;
        };
        if original_finding.rule != Rule::ModuleStateWrite {
            // Bundler wrappers can hide a global alias. Preserve the generated
            // span while adopting the original source's mutation classification.
            finding.rule = original_finding.rule;
            finding.target = original_finding.target.clone();
            finding.operation = original_finding.operation.clone();
            finding.confidence = original_finding.confidence;
            finding.detail = original_finding.detail.clone();
            finding.polyfill = original_finding.polyfill.clone();
            finding.declaration = None;
            return true;
        }
        if let (Some(declaration), Some(original)) = (&mut finding.declaration, original) {
            declaration.name = original.name.clone().unwrap();
            declaration.original = Some(original.clone());
            declaration.is_let = true;
        }
        true
    });
    // Apply this after refinement so global mutations recovered from original
    // sources retain their own rules, including inside dependencies.
    let dependency_resource = in_node_modules(&options.resource_url);
    analysis.findings.retain(|finding| {
        if finding.rule != Rule::ModuleStateWrite {
            return true;
        }
        let (line, column) = generated.position(finding.span.start);
        !dependency_resource
            && !map
                .and_then(|map| map.lookup_token(line - 1, column))
                .is_some_and(|token| token.get_source().is_some_and(in_node_modules))
    });
}

fn in_node_modules(resource: &str) -> bool {
    let path = resource.split(['?', '#']).next().unwrap_or_default();
    let url = url::Url::parse(path).ok();
    url.as_ref()
        .map_or(path, |url| url.path())
        .split(['/', '\\'])
        .any(|segment| segment == "node_modules")
}

impl<'s, 'c> Analyzer<'s, 'c> {
    fn module_binding(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        if self.options.script_mode != ScriptMode::Module || self.fn_stack.is_empty() {
            return None;
        }
        let symbol = self.symbol_of(id)?;
        let flags = self.scoping.symbol_flags(symbol);
        (self.scoping.symbol_scope_id(symbol) == self.scoping.root_scope_id()
            && flags.is_variable()
            && !flags.is_function())
        .then_some(symbol)
    }

    fn module_state_finding(
        &mut self,
        symbol: SymbolId,
        target: String,
        span: Span,
        operation: &str,
    ) {
        self.uncertain = false;
        let next = self.findings.len();
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
        if let Some(finding) = self.findings.get_mut(next) {
            let flags = self.scoping.symbol_flags(symbol);
            finding.declaration = Some(BindingDeclaration {
                name: self.scoping.symbol_name(symbol).to_string(),
                span: self.scoping.symbol_span(symbol).into(),
                is_let: flags.is_block_scoped() && !flags.is_const_variable(),
                original: None,
            });
        }
    }

    pub(super) fn module_state_reference(&mut self, id: &IdentifierReference<'_>) {
        if !id
            .reference_id
            .get()
            .is_some_and(|reference| self.scoping.get_reference(reference).is_write())
        {
            return;
        }
        if let Some(symbol) = self.module_binding(id) {
            let (span, operation) = self.write_context.unwrap_or((id.span.into(), "write"));
            self.module_state_finding(
                symbol,
                self.scoping.symbol_name(symbol).to_string(),
                span,
                operation,
            );
        }
    }

    /// Follow a property path to its lexical binding, without treating a
    /// same-named local variable or an imported binding as module-owned state.
    fn module_state_path(&mut self, mut expression: &Expression<'_>) -> Option<(SymbolId, String)> {
        let mut path: Vec<String> = vec![];
        loop {
            match expression.get_inner_expression() {
                Expression::Identifier(id) => {
                    let symbol = self.module_binding(id)?;
                    let mut target = self.scoping.symbol_name(symbol).to_string();
                    for key in path.iter().rev() {
                        target.push('.');
                        target.push_str(key);
                    }
                    return Some((symbol, target));
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
        if let Some((symbol, target)) = self.module_state_path(object) {
            self.module_state_finding(symbol, format!("{target}.{key}"), span, operation);
        }
    }
}
