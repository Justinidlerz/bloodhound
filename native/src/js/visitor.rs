use super::*;

impl<'s, 'a> Analyzer<'s, 'a> {
    pub(super) fn gap(&mut self, span: Span, reason: &str, cap: &str) {
        let item =
            json!({"start":span.start,"end":span.end,"reason":reason,"affectedCapabilities":[cap]});
        if !self.output.unresolved.contains(&item) {
            self.output.unresolved.push(item);
        }
    }
}
impl<'s, 'a> Visit<'a> for Analyzer<'s, 'a> {
    fn visit_expression(&mut self, e: &Expression<'a>) {
        self.visits += 1;
        if self.visits > self.options.max_steps {
            if self.visits == self.options.max_steps + 1 {
                self.gap(e.span(), "ast-visit-budget", "globals");
            }
            return;
        }
        if self.visits % 128 == 0 {
            let expired = self.options.deadline_unix_ms > 0
                && crate::model::now() >= self.options.deadline_unix_ms;
            let cancelled = !self.options.run_id.is_empty()
                && crate::cancellation(&self.options.run_id)
                    .load(std::sync::atomic::Ordering::Relaxed);
            if expired || cancelled {
                self.gap(
                    e.span(),
                    if cancelled {
                        "cancelled"
                    } else {
                        "analysis-time-budget"
                    },
                    "globals",
                );
                self.visits = self.options.max_steps + 1;
                return;
            }
        }
        walk::walk_expression(self, e);
    }

    fn visit_variable_declarator(&mut self, v: &VariableDeclarator<'a>) {
        self.inspect_variable_declarator(v);
    }
    fn visit_binding_identifier(&mut self, i: &BindingIdentifier<'a>) {
        self.inspect_binding_identifier(i);
    }
    fn visit_identifier_reference(&mut self, i: &IdentifierReference<'a>) {
        if self.suppressed.contains(&(i.span.start, i.span.end)) {
            return;
        }
        let v = if let Some(s) = self.symbol(i) {
            self.global_binding(s)
                .map(|t| Value::Targets(vec![t]))
                .unwrap_or(Value::Unknown)
        } else {
            self.root(i.name.as_str())
        };
        self.access(v, i.span, "read", "unknown", "semantic-reference");
    }
    fn visit_static_member_expression(&mut self, m: &StaticMemberExpression<'a>) {
        if !self.suppressed.contains(&(m.span.start, m.span.end)) {
            let b = self.object_target(&m.object);
            let v = self.prop(b, vec![m.property.name.to_string()], m.span);
            self.access(v, m.span, "read", "unknown", "member-and-alias-analysis");
        }
        self.visit_expression(&m.object);
    }
    fn visit_computed_member_expression(&mut self, m: &ComputedMemberExpression<'a>) {
        if !self.suppressed.contains(&(m.span.start, m.span.end)) {
            let b = self.object_target(&m.object);
            let k = self.eval(&m.expression);
            let v = self.computed(b, k, m.span);
            self.access(v, m.span, "read", "unknown", "finite-property-evaluation");
        }
        self.visit_expression(&m.object);
        self.visit_expression(&m.expression);
    }
    fn visit_assignment_expression(&mut self, a: &AssignmentExpression<'a>) {
        self.visit_expression(&a.right);
        let value = self.eval(&a.right);
        self.write_target(
            &a.left,
            if a.operator.as_str() == "=" {
                "write"
            } else {
                "compound"
            },
            value_type(&a.right),
        );
        self.visit_assignment_target(&a.left);
        match &a.left {
            AssignmentTarget::AssignmentTargetIdentifier(i) => {
                if let Some(s) = self.symbol(i) {
                    self.values.insert(
                        s,
                        if a.operator.as_str() == "=" {
                            value
                        } else {
                            Value::Unknown
                        },
                    );
                    self.functions.remove(&format!("s{s:?}"));
                    self.runtime_properties
                        .retain(|k, _| !k.starts_with(&format!("s{s:?}.")));
                } else {
                    self.blocked.insert(i.name.to_string());
                }
            }
            AssignmentTarget::StaticMemberExpression(m) => {
                if let Some(key) = self.expression_key(&m.object) {
                    let full = format!("{key}.{}", m.property.name);
                    self.register_function(full.clone(), &a.right);
                    if !matches!(value, Value::Unknown | Value::Typed("function")) {
                        self.runtime_properties.insert(full, value.clone());
                    }
                }
                if let Expression::Identifier(id) = &m.object {
                    if let Some(s) = self.symbol(id) {
                        let prop = m.property.name.to_string();
                        if let Some(Value::Node(tag)) = self.values.get(&s).cloned() {
                            if (tag == "script"
                                && ["text", "textContent", "innerHTML"].contains(&prop.as_str()))
                                || (tag == "iframe" && prop == "srcdoc")
                            {
                                let candidates = value.strings();
                                if candidates.is_empty() {
                                    self.gap(a.span, "non-finite-inline-code", "dependencies");
                                } else {
                                    for content in candidates {
                                        self.output.dependencies.push(json!({"start":a.span.start,"end":a.span.end,"kind":if tag=="iframe"{"iframe"}else{"script"},"scriptMode":"classic","environment":if tag=="iframe"{"window"}else{"inherit"},"specifiers":[],"content":content,"condition":"node-insertion-not-proven","sourceLocationApproximate":true}));
                                    }
                                }
                            }
                            if (tag == "script" && prop == "src")
                                || (tag == "link" && prop == "href")
                                || (tag == "iframe" && prop == "src")
                            {
                                if tag == "script" && value.strings().is_empty() {
                                    self.deferred_loads.push((
                                        self.alloc(&a.right),
                                        a.span,
                                        self.values.clone(),
                                    ));
                                } else {
                                    self.dep(
                                        value.clone(),
                                        a.span,
                                        if tag == "iframe" {
                                            "iframe"
                                        } else if tag == "link" {
                                            "style"
                                        } else {
                                            "script"
                                        },
                                        "classic",
                                        if tag == "iframe" { "window" } else { "inherit" },
                                    );
                                }
                            }
                        }
                        let entry = self
                            .values
                            .entry(s)
                            .or_insert_with(|| Value::Object(BTreeMap::new()));
                        if let Value::Object(map) = entry {
                            map.insert(prop, value);
                        }
                    }
                }
                let ts = self.eval(&m.object).targets();
                for t in ts {
                    if t.namespace == "builtin-property" {
                        self.blocked.insert(t.path_segments.join("."));
                    }
                }
            }
            AssignmentTarget::ComputedMemberExpression(m) => {
                let keys = self.eval(&m.expression).strings();
                if let Expression::Identifier(i) = &m.object {
                    if let Some(id) = self.symbol(i) {
                        if keys.len() == 1 {
                            if let Some(Value::Object(map)) = self.values.get_mut(&id) {
                                map.insert(keys[0].clone(), value);
                            }
                        } else {
                            self.values.remove(&id);
                        }
                    }
                }
            }
            _ => {
                self.gap(
                    a.left.span(),
                    "complex-assignment-propagation",
                    "evaluation",
                );
                self.invalidate_mutable();
            }
        }
    }
    fn visit_update_expression(&mut self, u: &UpdateExpression<'a>) {
        let v = match &u.argument {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(i) => {
                if let Some(s) = self.symbol(i) {
                    self.values.remove(&s);
                    self.global_binding(s)
                        .map(|t| Value::Targets(vec![t]))
                        .unwrap_or(Value::Unknown)
                } else {
                    Value::Targets(vec![Target::new(
                        "unbound-reference",
                        vec![i.name.to_string()],
                    )])
                }
            }
            _ => u
                .argument
                .as_member_expression()
                .map(|m| self.member_value(m))
                .unwrap_or(Value::Unknown),
        };
        self.access(v.clone(), u.span, "read", "unknown", "update");
        self.access(v, u.span, "write", "number", "update");
        self.suppressed
            .insert((u.argument.span().start, u.argument.span().end));
        walk::walk_update_expression(self, u);
    }
    fn visit_unary_expression(&mut self, u: &UnaryExpression<'a>) {
        if u.operator.as_str() == "delete" {
            let v = self.object_target(&u.argument);
            self.access(v, u.span, "delete", "undefined", "delete-expression");
            self.suppressed
                .insert((u.argument.span().start, u.argument.span().end));
        }
        walk::walk_unary_expression(self, u);
    }
    fn visit_function(&mut self, f: &Function<'a>, flags: ScopeFlags) {
        if let (Some(id), Some(body)) = (
            f.id.as_ref().and_then(|i| i.symbol_id.get()),
            f.body.as_ref(),
        ) {
            self.inspect_preload(id, body, &f.params);
        }
        let saved = self.values.clone();
        if saved.iter().any(|(id, v)| {
            !self.scoping.symbol_flags(*id).is_const_variable() && matches!(v, Value::Targets(_))
        }) {
            self.gap(f.span, "mutable-captured-alias", "globals");
        }
        self.invalidate_mutable();
        self.depth += 1;
        walk::walk_function(self, f, flags);
        self.depth -= 1;
        self.values = saved;
    }
    fn visit_arrow_function_expression(&mut self, f: &ArrowFunctionExpression<'a>) {
        let saved = self.values.clone();
        self.invalidate_mutable();
        self.depth += 1;
        walk::walk_arrow_function_expression(self, f);
        self.depth -= 1;
        self.values = saved;
    }
    fn visit_if_statement(&mut self, s: &IfStatement<'a>) {
        self.visit_expression(&s.test);
        let saved = self.values.clone();
        self.visit_statement(&s.consequent);
        let left = self.values.clone();
        self.values = saved;
        if let Some(alt) = &s.alternate {
            self.visit_statement(alt);
        }
        let right = self.values.clone();
        for (id, a) in left {
            let b = right.get(&id).cloned().unwrap_or(Value::Unknown);
            let val = self.join(a, b, s.span);
            self.values.insert(id, val);
        }
    }
    fn visit_conditional_expression(&mut self, e: &ConditionalExpression<'a>) {
        self.visit_expression(&e.test);
        let saved = self.values.clone();
        self.visit_expression(&e.consequent);
        let left = self.values.clone();
        self.values = saved;
        self.visit_expression(&e.alternate);
        let right = self.values.clone();
        for (id, a) in left {
            let b = right.get(&id).cloned().unwrap_or(Value::Unknown);
            let val = self.join(a, b, e.span);
            self.values.insert(id, val);
        }
    }
    fn visit_for_statement(&mut self, s: &ForStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_for_statement(self, s);
        self.invalidate_mutable();
    }
    fn visit_for_in_statement(&mut self, s: &ForInStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_for_in_statement(self, s);
        self.invalidate_mutable();
    }
    fn visit_for_of_statement(&mut self, s: &ForOfStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_for_of_statement(self, s);
        self.invalidate_mutable();
    }
    fn visit_while_statement(&mut self, s: &WhileStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_while_statement(self, s);
        self.invalidate_mutable();
    }
    fn visit_do_while_statement(&mut self, s: &DoWhileStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_do_while_statement(self, s);
        self.invalidate_mutable();
    }
    fn visit_switch_statement(&mut self, s: &SwitchStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_switch_statement(self, s);
        self.invalidate_mutable();
        self.gap(s.span, "switch-flow-not-propagated", "evaluation");
    }
    fn visit_try_statement(&mut self, s: &TryStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_try_statement(self, s);
        self.invalidate_mutable();
        self.gap(s.span, "exception-flow-not-propagated", "evaluation");
    }
    fn visit_with_statement(&mut self, s: &WithStatement<'a>) {
        self.gap(s.span, "with-dynamic-scope", "globals");
    }
    fn visit_import_declaration(&mut self, i: &ImportDeclaration<'a>) {
        self.inspect_import_declaration(i);
    }
    fn visit_export_named_declaration(&mut self, e: &ExportNamedDeclaration<'a>) {
        self.inspect_export_named_declaration(e);
    }
    fn visit_export_from_declaration(&mut self, e: &ExportFromDeclaration<'a>) {
        self.inspect_export_from_declaration(e);
    }
    fn visit_export_declaration(&mut self, e: &ExportDeclaration<'a>) {
        self.inspect_export_declaration(e);
    }
    fn visit_export_default_declaration(&mut self, e: &ExportDefaultDeclaration<'a>) {
        self.inspect_export_default_declaration(e);
    }
    fn visit_export_all_declaration(&mut self, e: &ExportAllDeclaration<'a>) {
        self.inspect_export_all_declaration(e);
    }
    fn visit_import_expression(&mut self, i: &ImportExpression<'a>) {
        self.inspect_import_expression(i);
    }
    fn visit_new_expression(&mut self, n: &NewExpression<'a>) {
        self.inspect_new_expression(n);
    }
    fn visit_call_expression(&mut self, c: &CallExpression<'a>) {
        self.inspect_call_expression(c);
    }
}
