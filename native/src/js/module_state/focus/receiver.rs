//! Structural candidates for state-container dispatch. Factory semantics and
//! instance lifetime remain unknown, so these candidates have medium confidence.

use super::*;

impl<'s, 'a> Flow<'s, 'a> {
    pub(super) fn prepare_receivers(&mut self) {
        let mut descriptors = HashMap::new();
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            if !self.step() {
                return;
            }
            let AstKind::ObjectExpression(object) = node.kind() else {
                continue;
            };
            // A descriptor supplied to a factory, not an arbitrary object or a
            // module-owned singleton on which init happens to be called.
            let AstKind::CallExpression(factory) = self.semantic.nodes().parent_kind(node.id())
            else {
                continue;
            };
            let mut descriptor_keys = HashSet::new();
            if !factory.arguments.iter().any(|a| a.span() == object.span)
                || object.properties.iter().any(|p| {
                    let ObjectPropertyKind::ObjectProperty(p) = p else {
                        return true;
                    };
                    p.computed
                        || p.key
                            .static_name()
                            .is_none_or(|key| !descriptor_keys.insert(key))
                })
            {
                continue;
            }
            let mut has_initial_object = false;
            let mut names = HashMap::<String, (usize, bool)>::new();
            for property in &object.properties {
                let ObjectPropertyKind::ObjectProperty(property) = property else {
                    continue;
                };
                let Some(value) = self.local_object(&property.value, 0) else {
                    continue;
                };
                let mut has_function = false;
                let mut opaque = false;
                for item in &value.properties {
                    let ObjectPropertyKind::ObjectProperty(item) = item else {
                        opaque = true;
                        continue;
                    };
                    let Some(name) = item.key.static_name().filter(|_| !item.computed) else {
                        opaque = true;
                        continue;
                    };
                    let function = self.resolve_function(&item.value, &Environment::new(), 0);
                    has_function |= function.is_some();
                    let entry = names.entry(name.into_owned()).or_insert((0, false));
                    entry.0 += 1;
                    entry.1 |= function.is_some_and(|f| self.merge_reducer(f));
                }
                if opaque {
                    // A spread or dynamic key could replace the reducer.
                    names.clear();
                    has_initial_object = false;
                    break;
                }
                has_initial_object |= !has_function;
            }
            let reducers: HashSet<_> = names
                .into_iter()
                .filter_map(|(name, (count, merge))| (count == 1 && merge).then_some(name))
                .collect();
            if has_initial_object && !reducers.is_empty() {
                descriptors.insert(node.id(), reducers);
            }
        }
        if descriptors.is_empty() {
            return;
        }
        for (call, _) in &self.calls {
            if !self.step() {
                return;
            }
            let Expression::StaticMemberExpression(member) = call.callee.get_inner_expression()
            else {
                continue;
            };
            if !matches!(
                member.object.get_inner_expression(),
                Expression::ThisExpression(_)
            ) || call.arguments.is_empty()
            {
                continue;
            }
            // Arrow callbacks inherit this; ordinary nested functions do not.
            let Some((root, _)) = self
                .semantic
                .nodes()
                .ancestors_enumerated(call.node_id.get())
                .find(|(_, n)| matches!(n.kind(), AstKind::Function(_)))
            else {
                continue;
            };
            let AstKind::ObjectProperty(method) = self.semantic.nodes().parent_kind(root) else {
                continue;
            };
            if function(&method.value) != Some(root) {
                continue;
            }
            for (ancestor, node) in self.semantic.nodes().ancestors_enumerated(root) {
                if let Some(reducers) = descriptors.get(&ancestor) {
                    if reducers.contains(member.property.name.as_str()) {
                        self.receiver_setters.insert(call.node_id.get(), root);
                    }
                    break;
                }
                if let AstKind::ObjectExpression(object) = node.kind() {
                    // A local method or an opaque spread can shadow the
                    // inferred factory-provided reducer dispatch.
                    if object.properties.iter().any(|p| {
                        !matches!(p, ObjectPropertyKind::ObjectProperty(p)
                            if !p.computed && p.key.static_name().is_some_and(|name| name != member.property.name.as_str()))
                    }) {
                        break;
                    }
                }
                // Only an arrow factory may wrap the action object. An ordinary
                // enclosing method establishes a different receiver boundary.
                if matches!(node.kind(), AstKind::Function(_)) {
                    break;
                }
            }
        }
    }

    fn local_object(
        &self,
        value: &'a Expression<'a>,
        depth: usize,
    ) -> Option<&'a ObjectExpression<'a>> {
        if depth > 12 || !self.step() {
            return None;
        }
        match value.get_inner_expression() {
            Expression::ObjectExpression(object) => Some(object),
            Expression::Identifier(id) => {
                let symbol = self.symbol(id)?;
                if !self.stable(symbol) {
                    return None;
                }
                self.local_object(self.definitions.get(&symbol)?, depth + 1)
            }
            _ => None,
        }
    }

    fn merge_reducer(&self, function: NodeId) -> bool {
        if matches!(self.semantic.nodes().kind(function), AstKind::Function(f) if f.r#async || f.generator)
            || matches!(self.semantic.nodes().kind(function), AstKind::ArrowFunctionExpression(f) if f.r#async)
        {
            return false;
        }
        let Some(params) = self.params(function) else {
            return false;
        };
        if params.items.len() != 2 || params.rest.is_some() {
            return false;
        }
        let mut symbols = Vec::new();
        for param in &params.items {
            let BindingPattern::BindingIdentifier(id) = &param.pattern else {
                return false;
            };
            let Some(symbol) = id.symbol_id.get() else {
                return false;
            };
            if !self.stable(symbol) {
                return false;
            }
            symbols.push(symbol);
        }
        // Exact state/payload merge evidence, independent of reducer, action,
        // factory and package names. Logging and opaque methods do not qualify.
        let returns = self.returns(function);
        if returns.len() != 1 {
            return false;
        }
        let Expression::ObjectExpression(object) = returns[0].get_inner_expression() else {
            return false;
        };
        if object.properties.len() != 2 {
            return false;
        }
        object.properties.iter().zip(symbols).all(|(p, expected)| {
            matches!(p, ObjectPropertyKind::SpreadProperty(p)
                if matches!(p.argument.get_inner_expression(), Expression::Identifier(i)
                    if self.symbol(i) == Some(expected)))
        })
    }
}
