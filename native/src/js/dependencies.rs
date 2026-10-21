use super::*;

impl<'s, 'a> Analyzer<'s, 'a> {
    pub(super) fn finish_loads(&mut self) {
        let loads = std::mem::take(&mut self.deferred_loads);
        for (expr, span, values) in loads {
            let saved = self.values.clone();
            self.values = values;
            let mut calls = LoadCalls::default();
            calls.visit_expression(expr);
            let mut candidates = Vec::new();
            let mut supported = false;
            for (root, property, arg) in calls.calls {
                let (Some(root), Some(arg)) = (
                    self.scoping.get_reference(root).symbol_id(),
                    self.scoping.get_reference(arg).symbol_id(),
                ) else {
                    continue;
                };
                let key = format!("s{root:?}.{property}");
                if let Some((_, body)) = self.functions.get(&key).cloned() {
                    let mut tables = TableKeys::default();
                    tables.visit_expression(body);
                    let root = key.split('.').next().unwrap_or("");
                    if let Some(hints) = self.load_hints.get(root) {
                        tables.keys.extend(hints.clone());
                    }
                    tables.keys.sort();
                    tables.keys.dedup();
                    if !tables.keys.is_empty() && tables.keys.len() <= self.options.max_candidates {
                        self.values.insert(arg, Value::Strings(tables.keys));
                        supported = true;
                    }
                }
            }
            if supported {
                if let (Some(public_path), Expression::BinaryExpression(b)) =
                    (self.options.public_path.clone(), expr)
                {
                    if let Expression::StaticMemberExpression(prefix) = &b.left {
                        if matches!(b.right, Expression::CallExpression(_)) {
                            if let Some(key) = self.expression_key(&b.left) {
                                self.runtime_properties
                                    .entry(key)
                                    .or_insert(Value::Strings(vec![public_path]));
                            }
                            let _ = prefix;
                        }
                    }
                }
                candidates = self.eval(expr).strings();
            }
            self.values = saved;
            if candidates.is_empty() {
                self.dep(Value::Unknown, span, "dynamic-script", "classic", "inherit");
            } else {
                self.dep(
                    Value::Strings(candidates),
                    span,
                    "webpack-chunk",
                    "classic",
                    "inherit",
                );
            }
        }
    }
    pub(super) fn inspect_preload(
        &mut self,
        id: SymbolId,
        body: &FunctionBody<'a>,
        params: &FormalParameters<'a>,
    ) {
        let mut shape = PreloadShape::default();
        shape.visit_function_body(body);
        let first = params.items.first().and_then(|p| {
            if let BindingPattern::BindingIdentifier(i) = &p.pattern {
                i.symbol_id.get()
            } else {
                None
            }
        });
        if first.is_some()
            && shape
                .links
                .iter()
                .any(|r| self.scoping.get_reference(*r).symbol_id().is_none())
            && shape
                .calls
                .iter()
                .any(|r| self.scoping.get_reference(*r).symbol_id() == first)
        {
            self.preload_helpers.insert(id);
        }
    }
}
#[derive(Default)]
pub(super) struct PreloadShape {
    pub(super) links: Vec<oxc_syntax::reference::ReferenceId>,
    pub(super) calls: Vec<oxc_syntax::reference::ReferenceId>,
}
impl<'a> Visit<'a> for PreloadShape {
    fn visit_call_expression(&mut self, c: &CallExpression<'a>) {
        if let Expression::Identifier(i) = &c.callee {
            if let Some(r) = i.reference_id.get() {
                self.calls.push(r);
            }
        }
        if let Expression::StaticMemberExpression(m) = &c.callee {
            if m.property.name == "createElement"
                && matches!(c.arguments.first().and_then(|a|a.as_expression()),Some(Expression::StringLiteral(s)) if s.value=="link")
            {
                if let Expression::Identifier(root) = &m.object {
                    if root.name == "document" {
                        if let Some(r) = root.reference_id.get() {
                            self.links.push(r);
                        }
                    }
                }
            }
        }
        walk::walk_call_expression(self, c);
    }
}
#[derive(Default)]
pub(super) struct LoadCalls {
    pub(super) calls: Vec<(
        oxc_syntax::reference::ReferenceId,
        String,
        oxc_syntax::reference::ReferenceId,
    )>,
}
impl<'a> Visit<'a> for LoadCalls {
    fn visit_call_expression(&mut self, c: &CallExpression<'a>) {
        if let Expression::StaticMemberExpression(m) = &c.callee {
            if let (Expression::Identifier(root), Some(Expression::Identifier(arg))) = (
                &m.object,
                c.arguments.first().and_then(|a| a.as_expression()),
            ) {
                // Resolve both references through Oxc before associating a runtime binding.
                if let (Some(root), Some(arg)) = (root.reference_id.get(), arg.reference_id.get()) {
                    self.calls.push((root, m.property.name.to_string(), arg));
                }
            }
        }
        walk::walk_call_expression(self, c);
    }
}
#[derive(Default)]
pub(super) struct TableKeys {
    pub(super) keys: Vec<String>,
}
impl<'a> Visit<'a> for TableKeys {
    fn visit_computed_member_expression(&mut self, m: &ComputedMemberExpression<'a>) {
        if let Expression::ObjectExpression(o) = &m.object {
            for p in &o.properties {
                if let ObjectPropertyKind::ObjectProperty(p) = p {
                    if let Some(k) = p.key.static_name() {
                        self.keys.push(k.into_owned());
                    }
                }
            }
        }
        walk::walk_computed_member_expression(self, m);
    }
}
