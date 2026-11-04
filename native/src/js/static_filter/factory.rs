use super::*;

impl<'s, 'a> Proof<'s, 'a> {
    pub(super) fn local_factories(&self) -> HashMap<SymbolId, Factory> {
        self.definitions
            .iter()
            .filter_map(|(symbol, e)| {
                if !self.stable(*symbol) {
                    return None;
                }
                Some((*symbol, self.factory(e)?))
            })
            .collect()
    }

    pub(super) fn exports(&self) -> HashMap<String, Factory> {
        let factories = self.local_factories();
        let mut exports = HashMap::new();
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            if !self.step() {
                break;
            }
            if let AstKind::ExportNamedDeclaration(e) = node.kind() {
                for specifier in &e.specifiers {
                    if let ModuleExportName::IdentifierReference(i) = &specifier.local {
                        if let Some(f) = self.symbol(i).and_then(|s| factories.get(&s)) {
                            exports.insert(specifier.exported.name().to_string(), f.clone());
                        }
                    }
                }
            }
        }
        exports
    }

    fn factory(&self, e: &Expression<'a>) -> Option<Factory> {
        // Match a factory returning a closure that calls one of its parameters.
        // The actual argument values, including fresh object assignments, are
        // summarized; no loader/export symbol spelling participates.
        let Expression::ArrowFunctionExpression(outer) = e.get_inner_expression() else {
            return None;
        };
        if outer.r#async {
            return None;
        }
        let Expression::ArrowFunctionExpression(inner) =
            outer.body.as_expression()?.get_inner_expression()
        else {
            return None;
        };
        if inner.r#async || !inner.params.items.is_empty() || inner.params.rest.is_some() {
            return None;
        }
        let parameter_symbols: Vec<_> = outer
            .params
            .items
            .iter()
            .map(|p| {
                if let BindingPattern::BindingIdentifier(i) = &p.pattern {
                    i.symbol_id.get()
                } else {
                    None
                }
            })
            .collect();
        let mut found = None;
        for (index, symbol) in parameter_symbols.iter().enumerate() {
            let Some(symbol) = symbol else {
                continue;
            };
            let mut call = None;
            let mut valid = true;
            for r in self.semantic.scoping().get_resolved_references(*symbol) {
                if !self.step() {
                    return None;
                }
                match self.semantic.nodes().parent_kind(r.node_id()) {
                    AstKind::CallExpression(c) if matches!(c.callee.get_inner_expression(), Expression::Identifier(i) if i.node_id.get()==r.node_id()) => {
                        if call.replace(c).is_some() {
                            valid = false;
                        }
                    }
                    // Clearing a completed callback is not an alternate value.
                    AstKind::AssignmentExpression(a)
                        if a.operator.as_str() == "="
                            && matches!(
                                a.right.get_inner_expression(),
                                Expression::NullLiteral(_)
                            ) => {}
                    _ => valid = false,
                }
            }
            let Some(call) = call.filter(|_| valid) else {
                continue;
            };
            if !(inner.span.start < call.span.start && call.span.end < inner.span.end) {
                continue;
            }
            let mut env = HashMap::new();
            let mut arguments = Vec::new();
            for arg in &call.arguments {
                let value =
                    self.factory_argument(arg.as_expression()?, &mut env, &parameter_symbols, 0);
                if !matches!(value, Fact::Object(_)) {
                    return None;
                }
                arguments.push(value);
            }
            if arguments.is_empty() || found.is_some() {
                return None;
            }
            found = Some(Factory {
                callback: index,
                arguments,
            });
        }
        found
    }

    fn factory_argument(
        &self,
        e: &Expression<'a>,
        env: &mut HashMap<SymbolId, Fact>,
        params: &[Option<SymbolId>],
        depth: usize,
    ) -> Fact {
        if depth > 16 || !self.step() {
            return Fact::Unknown;
        }
        match e.get_inner_expression() {
            Expression::Identifier(i) => self
                .symbol(i)
                .and_then(|s| env.get(&s))
                .cloned()
                .unwrap_or(Fact::Unknown),
            Expression::AssignmentExpression(a) if a.operator.as_str() == "=" => {
                let AssignmentTarget::AssignmentTargetIdentifier(i) = &a.left else {
                    return Fact::Unknown;
                };
                let Some(symbol) = self.symbol(i).filter(|s| params.contains(&Some(*s))) else {
                    return Fact::Unknown;
                };
                let value = self.factory_argument(&a.right, env, params, depth + 1);
                env.insert(symbol, value.clone());
                value
            }
            Expression::StaticMemberExpression(m) => {
                match self.factory_argument(&m.object, env, params, depth + 1) {
                    Fact::Object(object) => object
                        .get(m.property.name.as_str())
                        .cloned()
                        .unwrap_or(Fact::Unknown),
                    _ => Fact::Unknown,
                }
            }
            Expression::ObjectExpression(o) => {
                let mut object = BTreeMap::new();
                for p in &o.properties {
                    let ObjectPropertyKind::ObjectProperty(p) = p else {
                        return Fact::Unknown;
                    };
                    if p.computed || p.method || p.kind != PropertyKind::Init {
                        return Fact::Unknown;
                    }
                    let Some(key) = p.key.static_name() else {
                        return Fact::Unknown;
                    };
                    let value = self.factory_argument(&p.value, env, params, depth + 1);
                    if value == Fact::Unknown {
                        return Fact::Unknown;
                    }
                    object.insert(key.into_owned(), value);
                }
                Fact::Object(object)
            }
            _ => Fact::Unknown,
        }
    }

    pub(super) fn bind_factories(&mut self, factories: &HashMap<SymbolId, Factory>) {
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            if !self.step() {
                return;
            }
            let AstKind::CallExpression(c) = node.kind() else {
                continue;
            };
            let Expression::Identifier(i) = c.callee.get_inner_expression() else {
                continue;
            };
            let Some(f) = self.symbol(i).and_then(|s| factories.get(&s)) else {
                continue;
            };
            let Some(callback) = c
                .arguments
                .get(f.callback)
                .and_then(Argument::as_expression)
            else {
                continue;
            };
            let params = match callback.get_inner_expression() {
                Expression::ArrowFunctionExpression(f) => &f.params,
                Expression::FunctionExpression(f) => &f.params,
                _ => continue,
            };
            for (param, value) in params.items.iter().zip(&f.arguments) {
                if let BindingPattern::BindingIdentifier(i) = &param.pattern {
                    if let Some(symbol) = i.symbol_id.get() {
                        self.parameters.insert(symbol, value.clone());
                    }
                }
            }
        }
    }
}
