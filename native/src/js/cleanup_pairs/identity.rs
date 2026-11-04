use super::*;

impl<'s, 'a> Matcher<'s, 'a> {
    pub(super) fn step(&self) -> bool {
        let Some(n) = self.budget.get().checked_sub(1) else {
            return false;
        };
        self.budget.set(n);
        n % 256 != 0 || self.cancel.check().is_ok()
    }
    pub(super) fn symbol(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        id.reference_id
            .get()
            .and_then(|r| self.semantic.scoping().get_reference(r).symbol_id())
    }
    pub(super) fn prepare(&mut self) {
        let mut assignments: HashMap<SymbolId, Vec<(&'a Expression<'a>, Option<NodeId>, u32)>> =
            HashMap::new();
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            match node.kind() {
                AstKind::VariableDeclarator(d) => {
                    if let (BindingPattern::BindingIdentifier(id), Some(e)) = (&d.id, &d.init) {
                        if let Some(s) = id.symbol_id.get() {
                            assignments.entry(s).or_default().push((
                                e,
                                self.scope(d.node_id.get()),
                                d.span.start,
                            ));
                        }
                        // A named function expression's inner name denotes the same
                        // function object as the enclosing, immutable variable.
                        if let Expression::FunctionExpression(f) = e.get_inner_expression() {
                            if let (Some(outer), Some(inner)) = (
                                id.symbol_id.get(),
                                f.id.as_ref().and_then(|i| i.symbol_id.get()),
                            ) {
                                if !self
                                    .semantic
                                    .scoping()
                                    .get_resolved_references(outer)
                                    .any(|r| r.is_write())
                                {
                                    self.aliases.insert(inner, outer);
                                }
                            }
                        }
                    }
                }
                AstKind::AssignmentExpression(a) if a.operator.as_str() == "=" => {
                    if let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left {
                        if let Some(s) = self.symbol(id) {
                            assignments.entry(s).or_default().push((
                                &a.right,
                                self.scope(a.node_id.get()),
                                a.span.start,
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        for (s, values) in assignments {
            self.resets.insert(
                s,
                values
                    .iter()
                    .filter(|(e, _, _)| self.empty(e))
                    .map(|(_, scope, start)| (*scope, *start))
                    .collect(),
            );
            if values.len() == 1 {
                self.definitions.insert(s, values[0].0);
                continue;
            }
            let live: Vec<_> = values.iter().filter(|(e, _, _)| !self.empty(e)).collect();
            if live.len() == 1
                && values.iter().all(|(e, scope, start)| {
                    !self.empty(e) || *scope != live[0].1 || *start < live[0].2
                })
            {
                self.definitions.insert(s, live[0].0);
            } else {
                self.unstable.insert(s);
            }
        }
        for symbol in self.semantic.scoping().symbol_ids() {
            if self
                .semantic
                .scoping()
                .get_resolved_references(symbol)
                .any(|r| {
                    r.is_write()
                        && !matches!(
                            self.semantic.nodes().parent_kind(r.node_id()),
                            AstKind::AssignmentExpression(a) if a.operator.as_str() == "="
                        )
                })
            {
                self.unstable.insert(symbol);
            }
        }
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            let AstKind::CallExpression(c) = node.kind() else {
                continue;
            };
            let Some((object, "forEach")) = member(&c.callee) else {
                continue;
            };
            let Expression::ArrayExpression(array) = object.get_inner_expression() else {
                continue;
            };
            let params = match arg(c, 0).map(Expression::get_inner_expression) {
                Some(Expression::FunctionExpression(f)) => &f.params,
                Some(Expression::ArrowFunctionExpression(f)) => &f.params,
                _ => continue,
            };
            let Some(param) = params.items.first() else {
                continue;
            };
            if param.initializer.is_some() {
                continue;
            }
            let BindingPattern::BindingIdentifier(id) = &param.pattern else {
                continue;
            };
            let Some(symbol) = id.symbol_id.get() else {
                continue;
            };
            if self
                .semantic
                .scoping()
                .get_resolved_references(symbol)
                .any(|r| r.is_write())
            {
                continue;
            }
            let values = array
                .elements
                .iter()
                .map(|item| match item {
                    ArrayExpressionElement::StringLiteral(s) => {
                        Some(Key::Name(s.value.to_string()))
                    }
                    ArrayExpressionElement::TemplateLiteral(t) if t.expressions.is_empty() => t
                        .quasis
                        .first()
                        .and_then(|q| q.value.cooked.as_ref())
                        .map(|v| Key::Name(v.to_string())),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>();
            if let Some(values) = values {
                self.literal_iterations.insert(symbol, values);
            }
        }
        self.prepare_classes();
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            if let AstKind::MethodDefinition(m) = node.kind() {
                let (Some(instance), Some(name)) =
                    (self.instance(m.value.node_id.get()), m.key.static_name())
                else {
                    continue;
                };
                self.methods.insert(
                    instance.clone().property(&name),
                    (instance.clone(), m.value.span),
                );
                for (index, param) in m.value.params.items.iter().enumerate() {
                    if param.initializer.is_some() {
                        continue;
                    }
                    if let BindingPattern::BindingIdentifier(id) = &param.pattern {
                        if let Some(s) = id.symbol_id.get() {
                            if self
                                .semantic
                                .scoping()
                                .get_resolved_references(s)
                                .any(|r| r.is_write())
                            {
                                continue;
                            }
                            self.parameters
                                .insert(s, Key::Parameter(Box::new(instance.clone()), index));
                        }
                    }
                }
            }
        }
        let mut indices = Vec::new();
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            let candidate = match node.kind() {
                AstKind::AssignmentExpression(a) => match &a.left {
                    AssignmentTarget::ComputedMemberExpression(m) => {
                        self.key(&m.object, 0).zip(self.key(&m.expression, 0))
                    }
                    _ => None,
                },
                AstKind::CallExpression(c) => {
                    if let Some((object, "set")) = member(&c.callee) {
                        self.key(object, 0)
                            .zip(arg(c, 0).and_then(|e| self.key(e, 0)))
                    } else {
                        None
                    }
                }
                AstKind::ForInStatement(f) => {
                    if let ForStatementLeft::VariableDeclaration(d) = &f.left {
                        d.declarations
                            .first()
                            .and_then(|d| {
                                if let BindingPattern::BindingIdentifier(id) = &d.id {
                                    id.symbol_id.get().map(Key::Binding)
                                } else {
                                    None
                                }
                            })
                            .and_then(|key| self.key(&f.right, 0).map(|base| (base, key)))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some((base, key)) = candidate {
                if instance_root(&base) && matches!(key, Key::Parameter(..) | Key::Binding(_)) {
                    indices.push((key, Key::Index(Box::new(base))));
                }
            }
        }
        let mut ambiguous = HashSet::new();
        for (key, value) in indices {
            if self.indices.get(&key).is_some_and(|old| *old != value) {
                ambiguous.insert(key.clone());
            }
            self.indices.insert(key, value);
        }
        for key in ambiguous {
            self.indices.remove(&key);
        }
        // Captures and storage keep the computed-key relationship between
        // acquisition and release methods, including map-held callbacks.
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            match node.kind() {
                AstKind::VariableDeclarator(d) => {
                    if let (BindingPattern::BindingIdentifier(id), Some(e)) = (&d.id, &d.init) {
                        if let (Some(s), Some(value)) = (id.symbol_id.get(), self.key(e, 0)) {
                            if !self
                                .semantic
                                .scoping()
                                .get_resolved_references(s)
                                .any(|r| r.is_write())
                            {
                                self.snapshots.insert(
                                    Key::Binding(s),
                                    (value, d.span.start, self.scope(d.node_id.get())),
                                );
                            }
                        }
                    }
                }
                AstKind::AssignmentExpression(a) if a.operator.as_str() == "=" => {
                    if let (Some(slot), Some(value)) =
                        (self.left_key(&a.left), self.key(&a.right, 0))
                    {
                        if matches!(slot, Key::Member(..)) && matches!(value, Key::Binding(_)) {
                            self.stored.insert(value, slot);
                        }
                    }
                }
                AstKind::CallExpression(c) => {
                    if let Some((object, "set")) = member(&c.callee) {
                        if let (Some(base), Some(key), Some(value)) = (
                            self.key(object, 0),
                            arg(c, 0).and_then(|e| self.key(e, 0)),
                            arg(c, 1).and_then(|e| self.key(e, 0)),
                        ) {
                            let slot = base.member(key);
                            self.snapshots.insert(
                                slot.clone(),
                                (value.clone(), c.span.start, self.scope(c.node_id.get())),
                            );
                            if matches!(value, Key::Binding(_)) {
                                self.stored.insert(value, slot);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    fn empty(&self, e: &Expression<'a>) -> bool {
        match e.get_inner_expression() {
            Expression::NullLiteral(_) => true,
            Expression::Identifier(i) => i.name == "undefined" && self.symbol(i).is_none(),
            Expression::UnaryExpression(u) => {
                u.operator.as_str() == "void"
                    && matches!(
                        u.argument.get_inner_expression(),
                        Expression::NumericLiteral(_)
                    )
            }
            Expression::AssignmentExpression(a) if a.operator.as_str() == "=" => {
                self.empty(&a.right)
            }
            _ => false,
        }
    }
    pub(super) fn instance(&self, node: NodeId) -> Option<Key> {
        let nodes = self.semantic.nodes();
        for (id, item) in nodes.ancestors_enumerated(node) {
            if let AstKind::Class(c) = item.kind() {
                return Some(Key::Instance(c.span.start));
            }
            if let AstKind::Function(f) = item.kind() {
                if let Some(owner) = self.owners.get(&id) {
                    return Some(owner.clone());
                }
                if let AstKind::AssignmentExpression(a) = nodes.parent_kind(id) {
                    if let AssignmentTarget::StaticMemberExpression(m) = &a.left {
                        if let Expression::StaticMemberExpression(p) =
                            m.object.get_inner_expression()
                        {
                            if p.property.name == "prototype" {
                                return self.key(&p.object, 0).map(|k| k.property("instance"));
                            }
                        }
                    }
                }
                // Arrow closures retain their enclosing instance. Ordinary
                // functions with no class/prototype owner get distinct identities.
                if matches!(nodes.parent_kind(id), AstKind::MethodDefinition(_)) {
                    continue;
                }
                return Some(
                    f.id.as_ref()
                        .and_then(|i| i.symbol_id.get())
                        .map(|s| Key::Binding(s).property("instance"))
                        .unwrap_or(Key::Instance(f.span.start)),
                );
            }
        }
        None
    }
    pub(super) fn key(&self, e: &Expression<'a>, depth: usize) -> Option<Key> {
        if depth > 24 || !self.step() {
            return None;
        }
        match e.get_inner_expression() {
            Expression::Identifier(id) => {
                if let Some(s) = self.symbol(id) {
                    if self.unstable.contains(&s) {
                        return None;
                    }
                    if self.resets.get(&s).is_some_and(|resets| {
                        let scope = self.scope(id.node_id.get());
                        resets.iter().any(|(reset_scope, start)| {
                            *reset_scope == scope
                                && *start < id.span.start
                                && !self.definitions.get(&s).is_some_and(|value| {
                                    use oxc_span::GetSpan;
                                    let span = value.span();
                                    *start < span.start && span.end <= id.span.start
                                })
                        })
                    }) {
                        return None;
                    }
                    if let Some(owner) = self.returned_instances.get(&s) {
                        return Some(owner.clone());
                    }
                    if let Some(key) = self.parameters.get(&s) {
                        return Some(self.indices.get(key).unwrap_or(key).clone());
                    }
                    if let Some(key) = self.indices.get(&Key::Binding(s)) {
                        return Some(key.clone());
                    }
                    if let Some(alias) = self.aliases.get(&s) {
                        return Some(Key::Binding(*alias));
                    }
                    if let Some(init) = self.definitions.get(&s) {
                        if matches!(
                            init.get_inner_expression(),
                            Expression::Identifier(_)
                                | Expression::ThisExpression(_)
                                | Expression::StaticMemberExpression(_)
                                | Expression::ComputedMemberExpression(_)
                                | Expression::StringLiteral(_)
                                | Expression::TemplateLiteral(_)
                                | Expression::AssignmentExpression(_)
                                | Expression::CallExpression(_)
                                | Expression::ConditionalExpression(_)
                                | Expression::LogicalExpression(_)
                        ) {
                            if let Some(k) = self.key(init, depth + 1) {
                                return Some(k);
                            }
                        }
                    }
                    Some(Key::Binding(s))
                } else {
                    Some(Key::Name(
                        if matches!(id.name.as_str(), "window" | "globalThis" | "self") {
                            "window".into()
                        } else {
                            id.name.to_string()
                        },
                    ))
                }
            }
            Expression::ThisExpression(t) => self.instance(t.node_id.get()),
            Expression::StringLiteral(s) => Some(Key::Name(s.value.to_string())),
            Expression::NumericLiteral(n) => Some(Key::Name(n.value.to_string())),
            Expression::TemplateLiteral(t) if t.expressions.is_empty() => Some(Key::Name(
                t.quasis.first()?.value.cooked.as_ref()?.to_string(),
            )),
            Expression::StaticMemberExpression(m) => Some(
                self.key(&m.object, depth + 1)?
                    .property(m.property.name.as_str()),
            ),
            Expression::ComputedMemberExpression(m) => Some(
                self.key(&m.object, depth + 1)?
                    .member(self.key(&m.expression, depth + 1)?),
            ),
            Expression::AssignmentExpression(a) if a.operator.as_str() == "=" => {
                self.key(&a.right, depth + 1)
            }
            Expression::ConditionalExpression(c) => match self.browser_guard(&c.test, depth + 1) {
                Some(true) => self.key(&c.consequent, depth + 1),
                Some(false) => self.key(&c.alternate, depth + 1),
                None => Some(choice(
                    self.key(&c.consequent, depth + 1)?,
                    self.key(&c.alternate, depth + 1)?,
                )),
            },
            Expression::LogicalExpression(l) if matches!(l.operator.as_str(), "||" | "??") => {
                Some(choice(
                    self.key(&l.left, depth + 1)?,
                    self.key(&l.right, depth + 1)?,
                ))
            }
            Expression::CallExpression(c) => {
                let (receiver, method) = member(&c.callee)?;
                if method == "get" && c.arguments.len() == 1 {
                    Some(
                        self.key(receiver, depth + 1)?
                            .member(self.key(arg(c, 0)?, depth + 1)?),
                    )
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    pub(super) fn browser_guard(&self, e: &Expression<'a>, depth: usize) -> Option<bool> {
        if depth > 24 || !self.step() {
            return None;
        }
        match e.get_inner_expression() {
            Expression::Identifier(id) => self
                .definitions
                .get(&self.symbol(id)?)
                .and_then(|e| self.browser_guard(e, depth + 1)),
            Expression::LogicalExpression(l) if l.operator.as_str() == "&&" => Some(
                self.browser_guard(&l.left, depth + 1)?
                    && self.browser_guard(&l.right, depth + 1)?,
            ),
            Expression::BinaryExpression(b)
                if matches!(b.operator.as_str(), "!==" | "!=" | "===" | "==") =>
            {
                let (left, right) = if matches!(
                    b.left.get_inner_expression(),
                    Expression::UnaryExpression(_)
                ) {
                    (&b.left, &b.right)
                } else {
                    (&b.right, &b.left)
                };
                let Expression::UnaryExpression(u) = left.get_inner_expression() else {
                    return None;
                };
                if u.operator.as_str() != "typeof"
                    || !matches!(self.key(right,depth+1),Some(Key::Name(n)) if n=="undefined")
                {
                    return None;
                }
                let key = self.key(&u.argument, depth + 1)?;
                if key != Key::Name("window".into()) && key != Key::Name("document".into()) {
                    return None;
                }
                Some(matches!(b.operator.as_str(), "!==" | "!="))
            }
            _ => None,
        }
    }
}
