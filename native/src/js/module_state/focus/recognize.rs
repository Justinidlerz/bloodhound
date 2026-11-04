use super::*;

impl<'s, 'a> Flow<'s, 'a> {
    pub(super) fn recognize(&self) -> HashMap<SymbolId, (&'static str, Confidence)> {
        let mut selected = HashMap::new();
        let mut roots: HashSet<_> = self
            .calls
            .iter()
            .filter_map(|(c, owner)| {
                matches!(
                    self.hook(&c.callee, 0).as_deref(),
                    Some("useState" | "useReducer")
                )
                .then_some(*owner)
                .flatten()
            })
            .collect();
        // Resolve local custom-hook wrappers by actual calls, not their names.
        for _ in 0..12 {
            let mut extra = Vec::new();
            for (call, owner) in &self.calls {
                if self
                    .resolve_function(&call.callee, &Environment::new(), 0)
                    .is_some_and(|f| roots.contains(&f))
                {
                    if let Some(owner) = owner {
                        extra.push(*owner);
                    }
                }
            }
            let before = roots.len();
            roots.extend(extra);
            if before == roots.len() {
                break;
            }
        }
        roots.extend(self.receiver_setters.values().copied());
        for root in roots {
            if !self.step() {
                break;
            }
            let contexts = self.contexts(root, Environment::new());
            let functions: HashSet<_> = contexts.iter().map(|c| c.function).collect();
            for context in &contexts {
                for (call, owner) in &self.calls {
                    if *owner != Some(context.function) {
                        continue;
                    }
                    let hook = self.hook(&call.callee, 0);
                    let initial = match hook.as_deref() {
                        Some("useState") => argument(call, 0),
                        Some("useReducer") => argument(call, 1),
                        _ => None,
                    };
                    let Some(initial) = initial else {
                        continue;
                    };
                    let mut dependencies = self.dependencies(initial, &context.environment, 0);
                    let initializer = if hook.as_deref() == Some("useReducer") {
                        argument(call, 2)
                    } else {
                        Some(initial)
                    };
                    if let Some(function) =
                        initializer.and_then(|e| self.resolve_function(e, &context.environment, 0))
                    {
                        let arguments = if hook.as_deref() == Some("useReducer") {
                            vec![initial]
                        } else {
                            vec![]
                        };
                        let env = self.bind(function, &arguments, &context.environment);
                        dependencies.clear();
                        for returned in self.returns(function) {
                            dependencies.extend(self.dependencies(returned, &env, 0));
                        }
                    } else if hook.as_deref() == Some("useReducer")
                        && initializer.is_some_and(|e| {
                            self.literal(e, &context.environment, 0).as_deref() != Some("undefined")
                        })
                    {
                        // An opaque reducer initializer may discard its input.
                        dependencies.clear();
                    }
                    for symbol in dependencies {
                        if self.written_in(symbol, &functions)
                            && !self.fresh_context(context, call.span.start, symbol)
                            && !self.cleaned(symbol, &contexts)
                        {
                            selected.insert(symbol, (INITIAL_STATE, Confidence::High));
                        }
                    }
                }
                for (_, node) in self.semantic.nodes().iter_enumerated() {
                    if !self.step() {
                        break;
                    }
                    let AstKind::IfStatement(guard) = node.kind() else {
                        continue;
                    };
                    if self.owner(guard.node_id.get()) != Some(context.function)
                        || !self.empty_return(&guard.consequent)
                        || guard.alternate.is_some()
                    {
                        continue;
                    }
                    let dependencies = self.dependencies(&guard.test, &context.environment, 0);
                    if dependencies.is_empty() {
                        continue;
                    }
                    let updates = self.calls.iter().any(|(call, owner)| {
                        *owner == Some(context.function)
                            && call.span.start >= guard.span.end
                            && self.updates_state(call, &context.environment, &functions, 0)
                    });
                    if !updates {
                        continue;
                    }
                    for symbol in dependencies {
                        // A comparison made after refreshing the value in this
                        // invocation is a concurrency guard, not inherited state.
                        let receiver_root = self.receiver_setters.values().any(|f| *f == root);
                        let acquired_after_guard = !receiver_root
                            || self
                                .semantic
                                .scoping()
                                .get_resolved_references(symbol)
                                .any(|r| {
                                    r.is_write()
                                        && self.semantic.nodes().get_node(r.node_id()).span().start
                                            >= guard.span.end
                                        && self
                                            .owner(r.node_id())
                                            .is_some_and(|f| functions.contains(&f))
                                });
                        if acquired_after_guard
                            && self.written_in(symbol, &functions)
                            && !self.fresh_context(context, guard.span.start, symbol)
                            && !self.cleaned(symbol, &contexts)
                            && !self.settled_reset(symbol, context)
                        {
                            let evidence = if receiver_root {
                                (SKIPPED_RECEIVER_STATE, Confidence::Medium)
                            } else {
                                (SKIPPED_STATE, Confidence::High)
                            };
                            let previous = selected.entry(symbol).or_insert(evidence);
                            if evidence.1 == Confidence::High {
                                *previous = evidence;
                            }
                        }
                    }
                }
            }
        }
        selected
    }

    fn written_in(&self, symbol: SymbolId, functions: &HashSet<NodeId>) -> bool {
        self.semantic
            .scoping()
            .get_resolved_references(symbol)
            .any(|r| {
                r.is_write()
                    && self
                        .owner(r.node_id())
                        .is_some_and(|f| functions.contains(&f))
            })
    }

    fn empty_return(&self, statement: &Statement<'a>) -> bool {
        match statement {
            Statement::ReturnStatement(r) => r.argument.as_ref().is_none_or(|e| {
                self.literal(e, &Environment::new(), 0).as_deref() == Some("undefined")
            }),
            Statement::BlockStatement(b) if b.body.len() == 1 => self.empty_return(&b.body[0]),
            _ => false,
        }
    }

    fn updates_state(
        &self,
        call: &CallExpression<'a>,
        env: &Environment<'a>,
        roots: &HashSet<NodeId>,
        depth: usize,
    ) -> bool {
        if depth > 12 || !self.step() {
            return false;
        }
        if self
            .setter(&call.callee, env, 0)
            .is_some_and(|root| roots.contains(&root))
        {
            return true;
        }
        if self
            .receiver_setters
            .get(&call.node_id.get())
            .is_some_and(|root| roots.contains(root))
        {
            return true;
        }
        if let Some(function) = self.resolve_function(&call.callee, env, 0) {
            let arguments = call
                .arguments
                .iter()
                .filter_map(Argument::as_expression)
                .collect::<Vec<_>>();
            let next = self.bind(function, &arguments, env);
            return self.calls.iter().any(|(c, owner)| {
                *owner == Some(function) && self.updates_state(c, &next, roots, depth + 1)
            });
        }
        // Follow completion callbacks, not arbitrary external functions that
        // happen to accept a callback or contain a logging-like name.
        if let Expression::StaticMemberExpression(m) = call.callee.get_inner_expression() {
            if matches!(m.property.name.as_str(), "then" | "catch" | "finally") {
                return call
                    .arguments
                    .iter()
                    .filter_map(Argument::as_expression)
                    .any(|e| {
                        self.resolve_function(e, env, 0).is_some_and(|function| {
                            self.calls.iter().any(|(c, owner)| {
                                *owner == Some(function)
                                    && self.updates_state(c, env, roots, depth + 1)
                            })
                        })
                    });
            }
        }
        false
    }

    fn literal(&self, e: &Expression<'a>, env: &Environment<'a>, depth: usize) -> Option<String> {
        if depth > 16 || !self.step() {
            return None;
        }
        match e.get_inner_expression() {
            Expression::BooleanLiteral(b) => Some(b.value.to_string()),
            Expression::NullLiteral(_) => Some("null".into()),
            Expression::NumericLiteral(n) => Some(format!("number:{}", n.value)),
            Expression::StringLiteral(s) => Some(format!("string:{}", s.value)),
            Expression::Identifier(i) if i.name == "undefined" && self.symbol(i).is_none() => {
                Some("undefined".into())
            }
            Expression::Identifier(i) => {
                let s = self.symbol(i)?;
                if !self.stable(s) {
                    return None;
                }
                self.literal(
                    env.get(&s).or_else(|| self.definitions.get(&s))?,
                    env,
                    depth + 1,
                )
            }
            Expression::UnaryExpression(u)
                if u.operator.as_str() == "void"
                    && matches!(
                        u.argument.get_inner_expression(),
                        Expression::NumericLiteral(_)
                    ) =>
            {
                Some("undefined".into())
            }
            _ => None,
        }
    }

    fn cleaned(&self, symbol: SymbolId, contexts: &[Context<'a>]) -> bool {
        let initial = self.definitions.get(&symbol).map_or_else(
            || Some("undefined".into()),
            |e| self.literal(e, &Environment::new(), 0),
        );
        let Some(initial) = initial else {
            return false;
        };
        for context in contexts {
            for (call, owner) in &self.calls {
                if *owner != Some(context.function)
                    || !matches!(
                        self.hook(&call.callee, 0).as_deref(),
                        Some("useEffect" | "useLayoutEffect" | "useInsertionEffect")
                    )
                {
                    continue;
                }
                let Some(setup) = argument(call, 0)
                    .and_then(|e| self.resolve_function(e, &context.environment, 0))
                else {
                    continue;
                };
                for returned in self.returns(setup) {
                    let Some(cleanup) = self.resolve_function(returned, &context.environment, 0)
                    else {
                        continue;
                    };
                    if self.resets(cleanup, symbol, &initial, &context.environment, 0) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn resets(
        &self,
        function: NodeId,
        symbol: SymbolId,
        initial: &str,
        env: &Environment<'a>,
        depth: usize,
    ) -> bool {
        if depth > 12 || !self.step() || self.asynchronous(function) {
            return false;
        }
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            if self.owner(node.id()) != Some(function) {
                continue;
            }
            if let AstKind::AssignmentExpression(a) = node.kind() {
                if self.assignment_symbol(a) == Some(symbol)
                    && self.literal(&a.right, env, 0).as_deref() == Some(initial)
                {
                    return true;
                }
            }
        }
        self.calls.iter().any(|(call, owner)| {
            if *owner != Some(function) {
                return false;
            }
            let Some(f) = self.resolve_function(&call.callee, env, 0) else {
                return false;
            };
            let arguments = call
                .arguments
                .iter()
                .filter_map(Argument::as_expression)
                .collect::<Vec<_>>();
            self.resets(
                f,
                symbol,
                initial,
                &self.bind(f, &arguments, env),
                depth + 1,
            )
        })
    }

    fn assignment_symbol(&self, a: &AssignmentExpression<'a>) -> Option<SymbolId> {
        if a.operator.as_str() != "=" {
            return None;
        }
        if let AssignmentTarget::AssignmentTargetIdentifier(i) = &a.left {
            self.symbol(i)
        } else {
            None
        }
    }

    fn asynchronous(&self, function: NodeId) -> bool {
        matches!(self.semantic.nodes().kind(function), AstKind::Function(f) if f.r#async || f.generator)
            || matches!(self.semantic.nodes().kind(function), AstKind::ArrowFunctionExpression(f) if f.r#async)
    }

    fn fresh_value(&self, e: &Expression<'a>, env: &Environment<'a>, depth: usize) -> bool {
        if depth > 16 || !self.step() {
            return false;
        }
        if self.literal(e, env, depth).is_some() {
            return true;
        }
        match e.get_inner_expression() {
            Expression::Identifier(i) => {
                let Some(s) = self.symbol(i) else {
                    return false;
                };
                if self.candidates.contains(&s) || !self.stable(s) {
                    return false;
                }
                if let Some(value) = env.get(&s).or_else(|| self.definitions.get(&s)) {
                    return self.fresh_value(value, env, depth + 1);
                }
                self.semantic
                    .scoping()
                    .symbol_flags(s)
                    .is_function_scoped_declaration()
            }
            Expression::StaticMemberExpression(m) => self.fresh_value(&m.object, env, depth + 1),
            _ => false,
        }
    }

    fn fresh_before(
        &self,
        function: NodeId,
        before: u32,
        symbol: SymbolId,
        env: &Environment<'a>,
        depth: usize,
    ) -> bool {
        if depth > 12 || !self.step() {
            return false;
        }
        let Some(body) = self.body(function) else {
            return false;
        };
        for statement in &body.statements {
            if statement.span().end > before {
                break;
            }
            let Statement::ExpressionStatement(statement) = statement else {
                continue;
            };
            match statement.expression.get_inner_expression() {
                Expression::AssignmentExpression(a)
                    if self.assignment_symbol(a) == Some(symbol)
                        && self.fresh_value(&a.right, env, 0) =>
                {
                    return true;
                }
                Expression::CallExpression(c) => {
                    let Some(helper) = self.resolve_function(&c.callee, env, 0) else {
                        continue;
                    };
                    if self.asynchronous(helper) {
                        continue;
                    }
                    let arguments = c
                        .arguments
                        .iter()
                        .filter_map(Argument::as_expression)
                        .collect::<Vec<_>>();
                    // Only the straight-line prefix of a local helper can prove
                    // a write on every path returning to this caller.
                    if let Some(body) = self.body(helper) {
                        let end = body
                            .statements
                            .iter()
                            .find(|s| {
                                !matches!(
                                    s,
                                    Statement::ExpressionStatement(_)
                                        | Statement::VariableDeclaration(_)
                                )
                            })
                            .map_or(body.span.end, |s| s.span().start);
                        if self.fresh_before(
                            helper,
                            end,
                            symbol,
                            &self.bind(helper, &arguments, env),
                            depth + 1,
                        ) {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn fresh_context(&self, context: &Context<'a>, before: u32, symbol: SymbolId) -> bool {
        self.fresh_before(context.function, before, symbol, &context.environment, 0)
            || context.entries.iter().any(|(function, before)| {
                self.fresh_before(*function, *before, symbol, &context.environment, 0)
            })
    }

    fn settled_reset(&self, symbol: SymbolId, context: &Context<'a>) -> bool {
        let initial = self.definitions.get(&symbol).map_or_else(
            || Some("undefined".into()),
            |e| self.literal(e, &Environment::new(), 0),
        );
        let Some(initial) = initial else {
            return false;
        };
        let Some(body) = self.body(context.function) else {
            return false;
        };
        let mut regions = Vec::new();
        for (index, statement) in body.statements.iter().enumerate() {
            let Statement::TryStatement(t) = statement else {
                continue;
            };
            let Some(finalizer) = &t.finalizer else {
                continue;
            };
            let Some(Statement::ExpressionStatement(first)) = finalizer.body.first() else {
                continue;
            };
            let Expression::AssignmentExpression(reset) = first.expression.get_inner_expression()
            else {
                continue;
            };
            if self.assignment_symbol(reset) != Some(symbol)
                || self
                    .literal(&reset.right, &context.environment, 0)
                    .as_deref()
                    != Some(&initial)
            {
                continue;
            }
            regions.push(t.block.span);
            if let Some(Statement::ExpressionStatement(previous)) =
                index.checked_sub(1).and_then(|i| body.statements.get(i))
            {
                if let Expression::AssignmentExpression(a) =
                    previous.expression.get_inner_expression()
                {
                    if self.assignment_symbol(a) == Some(symbol) {
                        regions.push(a.span);
                    }
                }
            }
        }
        let mut acquired = false;
        for reference in self
            .semantic
            .scoping()
            .get_resolved_references(symbol)
            .filter(|r| r.is_write())
        {
            if !self.step() {
                return false;
            }
            let AstKind::AssignmentExpression(a) =
                self.semantic.nodes().parent_kind(reference.node_id())
            else {
                return false;
            };
            if self.assignment_symbol(a) != Some(symbol) {
                return false;
            }
            if self.literal(&a.right, &context.environment, 0).as_deref() == Some(&initial) {
                continue;
            }
            if self.owner(a.node_id.get()) != Some(context.function)
                || !regions
                    .iter()
                    .any(|region| region.start <= a.span.start && a.span.end <= region.end)
            {
                return false;
            }
            acquired = true;
        }
        acquired
    }
}
