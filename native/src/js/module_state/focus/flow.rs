use super::*;

impl<'s, 'a> Flow<'s, 'a> {
    pub(super) fn step(&self) -> bool {
        let Some(next) = self.budget.get().checked_sub(1) else {
            self.exhausted.set(true);
            return false;
        };
        self.budget.set(next);
        if next % 256 == 0 && self.cancel.check().is_err() {
            self.exhausted.set(true);
            return false;
        }
        true
    }

    pub(super) fn symbol(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        id.reference_id
            .get()
            .and_then(|r| self.semantic.scoping().get_reference(r).symbol_id())
    }

    pub(super) fn stable(&self, symbol: SymbolId) -> bool {
        !self
            .semantic
            .scoping()
            .get_resolved_references(symbol)
            .any(|r| r.is_write())
    }

    pub(super) fn owner(&self, node: NodeId) -> Option<NodeId> {
        self.semantic
            .nodes()
            .ancestors_enumerated(node)
            .find(|(_, n)| {
                matches!(
                    n.kind(),
                    AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
                )
            })
            .map(|(id, _)| id)
    }

    pub(super) fn body(&self, node: NodeId) -> Option<&'a FunctionBody<'a>> {
        match self.semantic.nodes().kind(node) {
            AstKind::Function(f) => f.body.as_deref(),
            AstKind::ArrowFunctionExpression(f) => f.body.as_function_body(),
            _ => None,
        }
    }

    pub(super) fn params(&self, node: NodeId) -> Option<&'a FormalParameters<'a>> {
        match self.semantic.nodes().kind(node) {
            AstKind::Function(f) => Some(&f.params),
            AstKind::ArrowFunctionExpression(f) => Some(&f.params),
            _ => None,
        }
    }

    pub(super) fn prepare(&mut self) {
        for symbol in self.semantic.scoping().symbol_ids() {
            let flags = self.semantic.scoping().symbol_flags(symbol);
            if flags.is_block_scoped()
                && !flags.is_const_variable()
                && flags.is_variable()
                && self.semantic.scoping().symbol_scope_id(symbol)
                    == self.semantic.scoping().root_scope_id()
            {
                self.candidates.insert(symbol);
            }
        }
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            if !self.step() {
                return;
            }
            match node.kind() {
                AstKind::ImportDeclaration(i) if i.source.value == "react" => {
                    for specifier in i.specifiers.iter().flatten() {
                        let (id, name) = match specifier {
                            ImportDeclarationSpecifier::ImportSpecifier(s) => {
                                (s.local.symbol_id.get(), s.imported.name().to_string())
                            }
                            ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                                (s.local.symbol_id.get(), "*".into())
                            }
                            ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                                (s.local.symbol_id.get(), "*".into())
                            }
                        };
                        if let Some(id) = id {
                            self.imports.insert(id, name);
                        }
                    }
                }
                AstKind::VariableDeclarator(d) => {
                    if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&d.id, &d.init) {
                        if let Some(symbol) = id.symbol_id.get() {
                            self.definitions.insert(symbol, init);
                            if let Some(f) = function(init) {
                                self.functions.insert(symbol, f);
                            }
                        }
                    }
                }
                AstKind::Function(f) if f.is_declaration() => {
                    if let Some(s) = f.id.as_ref().and_then(|id| id.symbol_id.get()) {
                        self.functions.insert(s, f.node_id.get());
                    }
                }
                AstKind::CallExpression(c) => self.calls.push((c, self.owner(c.node_id.get()))),
                _ => {}
            }
        }
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            let AstKind::VariableDeclarator(d) = node.kind() else {
                continue;
            };
            let (BindingPattern::ArrayPattern(p), Some(Expression::CallExpression(c))) =
                (&d.id, d.init.as_ref().map(Expression::get_inner_expression))
            else {
                continue;
            };
            if !matches!(
                self.hook(&c.callee, 0).as_deref(),
                Some("useState" | "useReducer")
            ) {
                continue;
            }
            let Some(Some(BindingPattern::BindingIdentifier(id))) = p.elements.get(1) else {
                continue;
            };
            if let (Some(symbol), Some(owner)) = (id.symbol_id.get(), self.owner(c.node_id.get())) {
                if self.stable(symbol) {
                    self.setters.insert(symbol, owner);
                }
            }
        }
    }

    pub(super) fn hook(&self, e: &Expression<'a>, depth: usize) -> Option<String> {
        if depth > 16 || !self.step() {
            return None;
        }
        match e.get_inner_expression() {
            Expression::Identifier(id) => {
                let symbol = self.symbol(id)?;
                if !self.stable(symbol) {
                    return None;
                }
                self.imports
                    .get(&symbol)
                    .cloned()
                    .or_else(|| self.hook(self.definitions.get(&symbol)?, depth + 1))
            }
            Expression::StaticMemberExpression(m)
                if self.hook(&m.object, depth + 1).as_deref() == Some("*") =>
            {
                Some(m.property.name.to_string())
            }
            Expression::ComputedMemberExpression(m)
                if self.hook(&m.object, depth + 1).as_deref() == Some("*") =>
            {
                if let Expression::StringLiteral(s) = m.expression.get_inner_expression() {
                    Some(s.value.to_string())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub(super) fn resolve_function(
        &self,
        e: &Expression<'a>,
        env: &Environment<'a>,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > 16 || !self.step() {
            return None;
        }
        if let Some(f) = function(e) {
            return Some(f);
        }
        let Expression::Identifier(id) = e.get_inner_expression() else {
            return None;
        };
        let symbol = self.symbol(id)?;
        if !self.stable(symbol) {
            return None;
        }
        if let Some(value) = env.get(&symbol) {
            return self.resolve_function(value, env, depth + 1);
        }
        self.functions
            .get(&symbol)
            .copied()
            .or_else(|| self.resolve_function(self.definitions.get(&symbol)?, env, depth + 1))
    }

    pub(super) fn setter(
        &self,
        e: &Expression<'a>,
        env: &Environment<'a>,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > 16 || !self.step() {
            return None;
        }
        let Expression::Identifier(id) = e.get_inner_expression() else {
            return None;
        };
        let symbol = self.symbol(id)?;
        if !self.stable(symbol) {
            return None;
        }
        if let Some(value) = env.get(&symbol) {
            return self.setter(value, env, depth + 1);
        }
        self.setters
            .get(&symbol)
            .copied()
            .or_else(|| self.setter(self.definitions.get(&symbol)?, env, depth + 1))
    }

    pub(super) fn bind(
        &self,
        function: NodeId,
        arguments: &[&'a Expression<'a>],
        env: &Environment<'a>,
    ) -> Environment<'a> {
        let mut next = env.clone();
        if let Some(params) = self.params(function) {
            for (index, param) in params.items.iter().enumerate() {
                if let BindingPattern::BindingIdentifier(id) = &param.pattern {
                    if let Some(symbol) = id.symbol_id.get() {
                        next.remove(&symbol);
                        if let Some(value) = arguments
                            .get(index)
                            .copied()
                            .or(param.initializer.as_deref())
                        {
                            next.insert(symbol, value);
                        }
                    }
                }
            }
        }
        next
    }

    pub(super) fn returns(&self, function: NodeId) -> Vec<&'a Expression<'a>> {
        if let AstKind::ArrowFunctionExpression(f) = self.semantic.nodes().kind(function) {
            if let Some(e) = f.body.as_expression() {
                return vec![e];
            }
        }
        self.semantic
            .nodes()
            .iter_enumerated()
            .filter_map(|(_, n)| {
                if let AstKind::ReturnStatement(r) = n.kind() {
                    if self.owner(r.node_id.get()) == Some(function) {
                        return r.argument.as_ref();
                    }
                }
                None
            })
            .collect()
    }

    pub(super) fn dependencies(
        &self,
        e: &Expression<'a>,
        env: &Environment<'a>,
        depth: usize,
    ) -> HashSet<SymbolId> {
        if depth > 16 || !self.step() {
            return HashSet::new();
        }
        let mut result = HashSet::new();
        let mut expressions = Vec::new();
        match e.get_inner_expression() {
            Expression::Identifier(id) => {
                if let Some(symbol) = self.symbol(id) {
                    if self.candidates.contains(&symbol) {
                        result.insert(symbol);
                    } else if self.stable(symbol) {
                        if let Some(value) =
                            env.get(&symbol).or_else(|| self.definitions.get(&symbol))
                        {
                            // A scalar snapshot captured during module evaluation
                            // is not refreshed by later writes to its source.
                            if !env.contains_key(&symbol)
                                && self.semantic.scoping().symbol_scope_id(symbol)
                                    == self.semantic.scoping().root_scope_id()
                            {
                                return result;
                            }
                            expressions.push(*value);
                        }
                    }
                }
            }
            Expression::BinaryExpression(b) => expressions.extend([&b.left, &b.right]),
            Expression::LogicalExpression(b) => expressions.extend([&b.left, &b.right]),
            Expression::UnaryExpression(u) => expressions.push(&u.argument),
            Expression::ConditionalExpression(c) => {
                expressions.extend([&c.test, &c.consequent, &c.alternate])
            }
            Expression::ObjectExpression(o) => {
                for property in &o.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = property {
                        expressions.push(&p.value);
                    }
                }
            }
            Expression::ArrayExpression(a) => {
                for element in &a.elements {
                    if let Some(e) = element.as_expression() {
                        expressions.push(e);
                    }
                }
            }
            Expression::CallExpression(c) => {
                if let Some(function) = self.resolve_function(&c.callee, env, depth + 1) {
                    let arguments = c
                        .arguments
                        .iter()
                        .filter_map(Argument::as_expression)
                        .collect::<Vec<_>>();
                    let next = self.bind(function, &arguments, env);
                    for returned in self.returns(function) {
                        result.extend(self.dependencies(returned, &next, depth + 1));
                    }
                }
            }
            _ => {}
        }
        for value in expressions {
            result.extend(self.dependencies(value, env, depth + 1));
        }
        result
    }

    pub(super) fn contexts(&self, root: NodeId, env: Environment<'a>) -> Vec<Context<'a>> {
        let mut result = Vec::new();
        let mut pending = vec![Context {
            function: root,
            environment: env,
            entries: Vec::new(),
            depth: 0,
        }];
        let mut seen = HashSet::new();
        while let Some(context) = pending.pop() {
            if !self.step() {
                break;
            }
            let mut bindings: Vec<_> = context
                .environment
                .iter()
                .map(|(s, e)| (*s, e.span().start))
                .collect();
            bindings.sort_unstable();
            if !seen.insert((context.function, bindings, context.entries.clone()))
                || context.depth > 12
            {
                continue;
            }
            if result.len() >= 256 {
                self.exhausted.set(true);
                break;
            }
            for (_, node) in self.semantic.nodes().iter_enumerated() {
                if matches!(
                    node.kind(),
                    AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
                ) && self.owner(node.id()) == Some(context.function)
                {
                    pending.push(Context {
                        function: node.id(),
                        environment: context.environment.clone(),
                        entries: context.entries.clone(),
                        depth: context.depth + 1,
                    });
                }
            }
            for (call, owner) in &self.calls {
                if *owner != Some(context.function) {
                    continue;
                }
                if let Some(function) = self.resolve_function(&call.callee, &context.environment, 0)
                {
                    let arguments = call
                        .arguments
                        .iter()
                        .filter_map(Argument::as_expression)
                        .collect::<Vec<_>>();
                    let mut entries = context.entries.clone();
                    entries.push((context.function, call.span.start));
                    pending.push(Context {
                        function,
                        environment: self.bind(function, &arguments, &context.environment),
                        entries,
                        depth: context.depth + 1,
                    });
                }
            }
            result.push(context);
        }
        result
    }
}
