use super::*;

impl<'s, 'a> Proof<'s, 'a> {
    fn owner(&self, node: NodeId) -> Option<NodeId> {
        self.semantic
            .nodes()
            .ancestors_enumerated(node)
            .find_map(|(id, n)| {
                matches!(
                    n.kind(),
                    AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
                )
                .then_some(id)
            })
    }

    fn immediate(&self, node: NodeId) -> bool {
        let span = self.semantic.nodes().get_node(node).span();
        self.semantic.nodes().ancestors_enumerated(node).find(|(_,n)|!matches!(n.kind(),AstKind::ParenthesizedExpression(_)))
            .is_some_and(|(_,n)|matches!(n.kind(),AstKind::CallExpression(c) if c.callee.get_inner_expression().span()==span))
    }

    fn synchronous_parameter_read(&self, symbol: SymbolId, reference: NodeId) -> bool {
        let declaration = self.owner(self.semantic.scoping().symbol_declaration(symbol));
        for (id, node) in self.semantic.nodes().ancestors_enumerated(reference) {
            if Some(id) == declaration {
                return true;
            }
            if matches!(
                node.kind(),
                AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
            ) && !self.immediate(id)
            {
                return false;
            }
        }
        false
    }

    /// Reject writes and opaque calls involving an object before the read.
    /// Mutation in a nested closure is also a possible mutation, not a reset.
    fn intact(&self, symbol: SymbolId, at: u32, object: bool) -> bool {
        self.semantic
            .scoping()
            .get_resolved_references(symbol)
            .all(|r| {
                if !self.step() {
                    return false;
                }
                let nodes = self.semantic.nodes();
                if nodes.get_node(r.node_id()).span().start >= at {
                    return true;
                }
                if r.is_write() {
                    return false;
                }
                for (_, parent) in nodes.ancestors_enumerated(r.node_id()) {
                    match parent.kind() {
                        AstKind::StaticMemberExpression(_)
                        | AstKind::ComputedMemberExpression(_)
                        | AstKind::ParenthesizedExpression(_) => {}
                        AstKind::AssignmentExpression(a) => {
                            return !object
                                && a.left.span().end < nodes.get_node(r.node_id()).span().start;
                        }
                        AstKind::UpdateExpression(_)
                        | AstKind::CallExpression(_)
                        | AstKind::NewExpression(_) => return false,
                        AstKind::UnaryExpression(u) if u.operator == UnaryOperator::Delete => {
                            return false;
                        }
                        AstKind::BinaryExpression(_)
                        | AstKind::LogicalExpression(_)
                        | AstKind::IfStatement(_)
                        | AstKind::ConditionalExpression(_)
                        | AstKind::UnaryExpression(_) => break,
                        _ => return !object,
                    }
                }
                true
            })
    }

    pub(super) fn stable(&self, symbol: SymbolId) -> bool {
        !self
            .semantic
            .scoping()
            .get_resolved_references(symbol)
            .any(|r| r.is_write())
    }

    pub(super) fn eval(&self, e: &Expression<'a>, at: u32, depth: usize) -> Fact {
        if depth > 24 || !self.step() {
            return Fact::Unknown;
        }
        let sub = |e| self.eval(e, at, depth + 1);
        match e.get_inner_expression() {
            Expression::Identifier(i) => {
                if let Some(symbol) = self.symbol(i) {
                    if let Some(value) = self.parameters.get(&symbol) {
                        return if self.synchronous_parameter_read(symbol, i.node_id.get())
                            && self.intact(symbol, at, true)
                        {
                            value.clone()
                        } else {
                            Fact::Unknown
                        };
                    }
                    if !self.stable(symbol) {
                        return Fact::Unknown;
                    }
                    let captured = self.owner(self.semantic.scoping().symbol_declaration(symbol))
                        != self.owner(i.node_id.get());
                    if captured
                        && !self
                            .semantic
                            .scoping()
                            .symbol_flags(symbol)
                            .is_const_variable()
                    {
                        return Fact::Unknown;
                    }
                    let Some(value) = self.definitions.get(&symbol) else {
                        return Fact::Unknown;
                    };
                    if value.span().end >= at {
                        return Fact::Unknown;
                    }
                    let value = sub(value);
                    if captured && matches!(value, Fact::Object(_)) {
                        return Fact::Unknown;
                    }
                    if matches!(
                        value,
                        Fact::Object(_) | Fact::Native(_) | Fact::MaybeNative(_)
                    ) && !self.intact(symbol, at, matches!(value, Fact::Object(_)))
                    {
                        return Fact::Unknown;
                    }
                    value
                } else if i.name == "undefined" {
                    Fact::Undefined
                } else {
                    builtins::classify(&i.name)
                        .map(|r| Fact::Native(r.display_name().into()))
                        .unwrap_or(Fact::Unknown)
                }
            }
            Expression::BooleanLiteral(b) => Fact::Bool(b.value),
            Expression::NumericLiteral(n) => Fact::Number(n.value),
            Expression::StringLiteral(s) => Fact::String(s.value.to_string()),
            Expression::NullLiteral(_) => Fact::Null,
            Expression::ThisExpression(this) => {
                for (_, node) in self
                    .semantic
                    .nodes()
                    .ancestors_enumerated(this.node_id.get())
                {
                    let AstKind::Function(f) = node.kind() else {
                        continue;
                    };
                    let strict = self.semantic.scoping().root_scope_flags().is_strict_mode()
                        || f.body.as_ref().is_some_and(|b| {
                            b.directives.iter().any(|d| d.directive == "use strict")
                        });
                    let called_directly = self.semantic.nodes().ancestors_enumerated(node.id()).find(|(_,n)| !matches!(n.kind(),AstKind::ParenthesizedExpression(_)))
                        .is_some_and(|(_,n)| matches!(n.kind(),AstKind::CallExpression(c) if matches!(c.callee.get_inner_expression(),Expression::FunctionExpression(callee) if callee.node_id.get()==f.node_id.get())));
                    return if strict && called_directly {
                        Fact::Undefined
                    } else {
                        Fact::Unknown
                    };
                }
                Fact::Unknown
            }
            Expression::TemplateLiteral(t) if t.expressions.is_empty() => t
                .quasis
                .first()
                .and_then(|q| q.value.cooked.as_ref())
                .map(|s| Fact::String(s.to_string()))
                .unwrap_or(Fact::Unknown),
            Expression::ObjectExpression(o) => {
                let mut values = BTreeMap::new();
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
                    values.insert(key.into_owned(), sub(&p.value));
                }
                Fact::Object(values)
            }
            Expression::StaticMemberExpression(m) => {
                member(sub(&m.object), m.property.name.as_str())
            }
            Expression::ComputedMemberExpression(m) => match sub(&m.expression) {
                Fact::String(key) => member(sub(&m.object), &key),
                _ => Fact::Unknown,
            },
            Expression::UnaryExpression(u) => match u.operator {
                UnaryOperator::Void => Fact::Undefined,
                UnaryOperator::LogicalNot => sub(&u.argument)
                    .truth()
                    .map(|v| Fact::Bool(!v))
                    .unwrap_or(Fact::Unknown),
                UnaryOperator::Typeof => match sub(&u.argument) {
                    Fact::Undefined => Fact::String("undefined".into()),
                    Fact::Null | Fact::Object(_) => Fact::String("object".into()),
                    Fact::Bool(_) => Fact::String("boolean".into()),
                    Fact::Number(_) => Fact::String("number".into()),
                    Fact::String(_) => Fact::String("string".into()),
                    _ => Fact::Unknown,
                },
                _ => Fact::Unknown,
            },
            Expression::BinaryExpression(b) => {
                let left = sub(&b.left);
                let right = sub(&b.right);
                match b.operator {
                    BinaryOperator::StrictEquality
                    | BinaryOperator::StrictInequality
                    | BinaryOperator::Equality
                    | BinaryOperator::Inequality => {
                        let equal = match (&left, &right) {
                            (Fact::Unknown | Fact::Native(_) | Fact::MaybeNative(_), _)
                            | (_, Fact::Unknown | Fact::Native(_) | Fact::MaybeNative(_))
                            | (Fact::Object(_), Fact::Object(_)) => None,
                            (Fact::Object(_), Fact::Undefined | Fact::Null)
                            | (Fact::Undefined | Fact::Null, Fact::Object(_)) => Some(false),
                            (Fact::String(_), Fact::String(_))
                            | (Fact::Number(_), Fact::Number(_))
                            | (Fact::Bool(_), Fact::Bool(_))
                            | (Fact::Undefined, Fact::Undefined)
                            | (Fact::Null, Fact::Null) => Some(left == right),
                            _ if matches!(
                                b.operator,
                                BinaryOperator::StrictEquality | BinaryOperator::StrictInequality
                            ) =>
                            {
                                Some(false)
                            }
                            _ => None,
                        };
                        equal
                            .map(|v| {
                                Fact::Bool(
                                    v == matches!(
                                        b.operator,
                                        BinaryOperator::StrictEquality | BinaryOperator::Equality
                                    ),
                                )
                            })
                            .unwrap_or(Fact::Unknown)
                    }
                    BinaryOperator::Addition => match (left, right) {
                        (Fact::String(a), Fact::String(b)) => Fact::String(a + &b),
                        _ => Fact::Unknown,
                    },
                    _ => Fact::Unknown,
                }
            }
            Expression::LogicalExpression(l) => {
                let left = sub(&l.left);
                let right = sub(&l.right);
                match l.operator {
                    LogicalOperator::And => match left.truth() {
                        Some(false) => left,
                        Some(true) => right,
                        _ => Fact::Unknown,
                    },
                    LogicalOperator::Or => match left.truth() {
                        Some(true) => left,
                        Some(false) => right,
                        _ => Fact::Unknown,
                    },
                    LogicalOperator::Coalesce => match left.nullish() {
                        Some(false) => left,
                        Some(true) => right,
                        _ => Fact::Unknown,
                    },
                }
            }
            Expression::ConditionalExpression(c) => match sub(&c.test).truth() {
                Some(true) => sub(&c.consequent),
                Some(false) => sub(&c.alternate),
                None => {
                    let a = sub(&c.consequent);
                    let b = sub(&c.alternate);
                    if a == b {
                        a
                    } else {
                        match (a, b) {
                            (
                                Fact::Native(n) | Fact::MaybeNative(n),
                                Fact::Null | Fact::Undefined,
                            )
                            | (
                                Fact::Null | Fact::Undefined,
                                Fact::Native(n) | Fact::MaybeNative(n),
                            ) => Fact::MaybeNative(n),
                            (Fact::Native(a), Fact::MaybeNative(b))
                            | (Fact::MaybeNative(a), Fact::Native(b))
                                if a == b =>
                            {
                                Fact::MaybeNative(a)
                            }
                            _ => Fact::Unknown,
                        }
                    }
                }
            },
            _ => Fact::Unknown,
        }
    }
}

fn member(object: Fact, key: &str) -> Fact {
    match object {
        Fact::Object(map) => map.get(key).cloned().unwrap_or(Fact::Unknown),
        Fact::Native(root) | Fact::MaybeNative(root) if root == "window" => builtins::classify(key)
            .map(|r| Fact::Native(r.display_name().into()))
            .unwrap_or_else(|| Fact::Native(format!("window.{key}"))),
        Fact::Native(root) | Fact::MaybeNative(root) => Fact::Native(format!("{root}.{key}")),
        _ => Fact::Unknown,
    }
}
