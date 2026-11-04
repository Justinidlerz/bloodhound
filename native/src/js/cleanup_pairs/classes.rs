use super::*;

impl<'s, 'a> Matcher<'s, 'a> {
    pub(super) fn prepare_classes(&mut self) {
        let nodes = self.semantic.nodes();
        let functions: HashMap<_, _> = nodes
            .iter_enumerated()
            .filter_map(|(id, n)| {
                if let AstKind::Function(f) = n.kind() {
                    f.id.as_ref()
                        .and_then(|i| i.symbol_id.get())
                        .map(|s| (s, (id, f)))
                } else {
                    None
                }
            })
            .collect();
        let mut constructors = HashSet::new();
        for (_, node) in nodes.iter_enumerated() {
            let AstKind::CallExpression(c) = node.kind() else {
                continue;
            };
            let Expression::Identifier(callee) = c.callee.get_inner_expression() else {
                continue;
            };
            let Some(helper) = self.symbol(callee) else {
                continue;
            };
            let Some(Expression::Identifier(ctor)) =
                arg(c, 0).map(Expression::get_inner_expression)
            else {
                continue;
            };
            let Some(constructor) = self.symbol(ctor) else {
                continue;
            };
            if !functions.contains_key(&constructor)
                || !self.defines_properties(helper, &functions, &mut HashSet::new())
            {
                continue;
            }
            for (index, arg) in c.arguments.iter().enumerate().skip(1) {
                let Some(Expression::ArrayExpression(array)) =
                    arg.as_expression().map(Expression::get_inner_expression)
                else {
                    continue;
                };
                let owner = if index == 1 {
                    Key::Binding(constructor).property("instance")
                } else {
                    Key::Binding(constructor)
                };
                for element in &array.elements {
                    let ArrayExpressionElement::ObjectExpression(descriptor) = element else {
                        continue;
                    };
                    let name = descriptor.properties.iter().find_map(|p| {
                        if let ObjectPropertyKind::ObjectProperty(p) = p {
                            (p.key.static_name().as_deref() == Some("key"))
                                .then(|| self.key(&p.value, 0))
                                .flatten()
                        } else {
                            None
                        }
                    });
                    let Some(Key::Name(name)) = name else {
                        continue;
                    };
                    for property in &descriptor.properties {
                        let ObjectPropertyKind::ObjectProperty(p) = property else {
                            continue;
                        };
                        if p.key.static_name().as_deref() != Some("value") {
                            continue;
                        }
                        if let Expression::FunctionExpression(f) = p.value.get_inner_expression() {
                            self.owners.insert(f.node_id.get(), owner.clone());
                            self.methods
                                .insert(owner.clone().property(&name), (owner.clone(), f.span));
                        }
                    }
                }
                constructors.insert(constructor);
            }
        }
        for constructor in constructors {
            let (id, function) = functions[&constructor];
            let owner = Key::Binding(constructor).property("instance");
            self.owners.insert(id, owner.clone());
            for (_, node) in nodes.iter_enumerated() {
                if let AstKind::ReturnStatement(r) = node.kind() {
                    if !(function.span.start <= r.span.start && r.span.end <= function.span.end) {
                        continue;
                    }
                    let nearest = nodes
                        .ancestors_enumerated(r.node_id.get())
                        .find(|(_, n)| {
                            matches!(
                                n.kind(),
                                AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
                            )
                        })
                        .map(|(id, _)| id);
                    if nearest != Some(id) {
                        continue;
                    }
                    if let Some(Expression::Identifier(i)) =
                        r.argument.as_ref().map(Expression::get_inner_expression)
                    {
                        if let Some(s) = self.symbol(i) {
                            self.returned_instances.insert(s, owner.clone());
                        }
                    }
                }
            }
        }
    }
    pub(super) fn defines_properties(
        &self,
        symbol: SymbolId,
        functions: &HashMap<SymbolId, (NodeId, &Function<'a>)>,
        seen: &mut HashSet<SymbolId>,
    ) -> bool {
        if seen.len() > 8 || !seen.insert(symbol) || !self.step() {
            return false;
        }
        let function = functions.get(&symbol).map(|(_, f)| *f).or_else(|| {
            let init = self.definitions.get(&symbol)?.get_inner_expression();
            let expression = if let Expression::CallExpression(c) = init {
                c.callee.get_inner_expression()
            } else {
                init
            };
            if let Expression::FunctionExpression(f) = expression {
                Some(f.as_ref())
            } else {
                None
            }
        });
        let Some(function) = function else {
            return false;
        };
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            let AstKind::CallExpression(c) = node.kind() else {
                continue;
            };
            if !(function.span.start <= c.span.start && c.span.end <= function.span.end) {
                continue;
            }
            if let Some((object, "defineProperty" | "defineProperties")) = member(&c.callee) {
                if matches!(object.get_inner_expression(),Expression::Identifier(i) if i.name=="Object"&&self.symbol(i).is_none())
                {
                    return true;
                }
            }
            if let Expression::Identifier(id) = c.callee.get_inner_expression() {
                if let Some(s) = self.symbol(id) {
                    if self.unstable.contains(&s) {
                        return false;
                    }
                    if self.defines_properties(s, functions, seen) {
                        return true;
                    }
                }
            }
        }
        false
    }
}
