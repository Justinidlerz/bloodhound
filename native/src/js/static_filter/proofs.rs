use super::*;

type Changes = (
    Vec<Span>,
    Vec<(Rule, Span)>,
    HashMap<Span, (String, String)>,
);

fn contains(outer: oxc_span::Span, inner: oxc_span::Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

impl<'s, 'a> Proof<'s, 'a> {
    pub(super) fn changes(&self) -> Changes {
        let mut dead = Vec::new();
        let mut remove = Vec::new();
        let mut polyfills = HashMap::new();
        for (_, node) in self.semantic.nodes().iter_enumerated() {
            if !self.step() {
                break;
            }
            match node.kind() {
                AstKind::IfStatement(i) => match self.eval(&i.test, i.test.span().start, 0).truth()
                {
                    Some(false) => dead.push(i.consequent.span().into()),
                    Some(true) => {
                        if let Some(a) = &i.alternate {
                            dead.push(a.span().into());
                        }
                    }
                    _ => {}
                },
                AstKind::ConditionalExpression(c) => {
                    match self.eval(&c.test, c.test.span().start, 0).truth() {
                        Some(false) => dead.push(c.consequent.span().into()),
                        Some(true) => dead.push(c.alternate.span().into()),
                        _ => {}
                    }
                }
                AstKind::LogicalExpression(l) => {
                    let left = self.eval(&l.left, l.left.span().start, 0);
                    let skip = match l.operator {
                        LogicalOperator::And => left.truth() == Some(false),
                        LogicalOperator::Or => left.truth() == Some(true),
                        LogicalOperator::Coalesce => left.nullish() == Some(false),
                    };
                    if skip {
                        dead.push(l.right.span().into());
                    }
                }
                AstKind::CallExpression(c) => {
                    let method = match c.callee.get_inner_expression() {
                        Expression::StaticMemberExpression(m) => Some(m.property.name.as_str()),
                        Expression::Identifier(i) if self.symbol(i).is_none() => {
                            Some(i.name.as_str())
                        }
                        _ => None,
                    };
                    if method == Some("addEventListener")
                        && c.arguments
                            .get(1)
                            .and_then(Argument::as_expression)
                            .is_some_and(|e| {
                                matches!(
                                    self.eval(e, c.span.start, 0),
                                    Fact::Null | Fact::Undefined
                                )
                            })
                    {
                        // Only the listener finding is removed. Argument/getter
                        // effects and all dependencies remain in the analysis.
                        remove.push((Rule::GlobalListener, c.span.into()));
                    }
                }
                AstKind::AssignmentExpression(a) => {
                    if let AssignmentTarget::ComputedMemberExpression(m) = &a.left {
                        if matches!(self.eval(&m.object,a.span.start,0),Fact::Native(n) if n=="localStorage" || n=="sessionStorage")
                            && self.data_key(&m.expression, a.span.start, 0)
                        {
                            remove.push((Rule::HostApiOverride, a.span.into()));
                        }
                        if a.operator.as_str() == "=" {
                            if let Some(target) = self.symbol_install(a, m) {
                                polyfills.insert(a.span.into(), (target.clone(), target));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        (dead, remove, polyfills)
    }

    fn data_key(&self, e: &Expression<'a>, at: u32, depth: usize) -> bool {
        if depth > 24 || !self.step() {
            return false;
        }
        if matches!(self.eval(e,at,0),Fact::String(s) if s.contains('.')) {
            return true;
        }
        match e.get_inner_expression() {
            Expression::BinaryExpression(b) if b.operator == BinaryOperator::Addition => {
                self.data_key(&b.left, at, depth + 1) || self.data_key(&b.right, at, depth + 1)
            }
            Expression::TemplateLiteral(t) => t
                .quasis
                .iter()
                .any(|q| q.value.cooked.as_ref().is_some_and(|s| s.contains('.'))),
            Expression::Identifier(i) => self
                .symbol(i)
                .filter(|s| self.stable(*s))
                .and_then(|s| self.definitions.get(&s))
                .is_some_and(|e| e.span().end < at && self.data_key(e, at, depth + 1)),
            _ => false,
        }
    }

    /// Resolve the well-known tag key, including availability-guarded aliases.
    /// A short-circuit alias additionally needs a truthy guard at the write.
    fn tag_key(&self, e: &Expression<'a>, at: u32, depth: usize) -> Option<bool> {
        if depth > 24 || !self.step() {
            return None;
        }
        if self.eval(e, at, 0) == Fact::Native("Symbol.toStringTag".into()) {
            return Some(false);
        }
        match e.get_inner_expression() {
            Expression::Identifier(i) => {
                let s = self.symbol(i).filter(|s| self.stable(*s))?;
                let init = self.definitions.get(&s)?;
                (init.span().end < at).then_some(())?;
                self.tag_key(init, at, depth + 1)
            }
            Expression::LogicalExpression(l)
                if l.operator == LogicalOperator::And
                    && self.eval(&l.left, at, 0) == Fact::Native("Symbol".into()) =>
            {
                self.tag_key(&l.right, at, depth + 1)?;
                Some(true)
            }
            _ => None,
        }
    }

    fn tag_target(&self, e: &Expression<'a>, at: u32) -> Option<String> {
        let Expression::ComputedMemberExpression(m) = e.get_inner_expression() else {
            return None;
        };
        self.tag_key(&m.expression, at, 0)?;
        self.tag_receiver(&m.object, at)
    }

    fn tag_receiver(&self, e: &Expression<'a>, at: u32) -> Option<String> {
        let Fact::Native(receiver) = self.eval(e, at, 0) else {
            return None;
        };
        let name = receiver.strip_suffix(".prototype")?;
        // Standard host/builtin prototypes only, not a custom global object.
        match builtins::classify(name)? {
            super::super::values::Root::Builtin(_) | super::super::values::Root::Host(_) => {
                Some(format!("{receiver}[Symbol.toStringTag]"))
            }
            _ => None,
        }
    }

    fn missing_tag(
        &self,
        e: &Expression<'a>,
        truth: bool,
        target: &str,
        at: u32,
        depth: usize,
    ) -> bool {
        if depth > 24 || !self.step() {
            return false;
        }
        if !truth && self.tag_target(e, at).as_deref() == Some(target) {
            return true;
        }
        match e.get_inner_expression() {
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::LogicalNot => {
                self.missing_tag(&u.argument, !truth, target, at, depth + 1)
            }
            Expression::LogicalExpression(l)
                if (l.operator == LogicalOperator::And && truth)
                    || (l.operator == LogicalOperator::Or && !truth) =>
            {
                self.missing_tag(&l.left, truth, target, at, depth + 1)
                    || self.missing_tag(&l.right, truth, target, at, depth + 1)
            }
            _ => false,
        }
    }

    fn same_key(&self, a: &Expression<'a>, b: &Expression<'a>) -> bool {
        matches!((a.get_inner_expression(),b.get_inner_expression()),(Expression::Identifier(a),Expression::Identifier(b)) if self.symbol(a).is_some() && self.symbol(a)==self.symbol(b))
    }

    fn symbol_install(
        &self,
        a: &AssignmentExpression<'a>,
        m: &ComputedMemberExpression<'a>,
    ) -> Option<String> {
        if !matches!(self.eval(&a.right, a.span.start, 0), Fact::String(_)) {
            return None;
        }
        let guarded_key = self.tag_key(&m.expression, a.span.start, 0)?;
        let target = self.tag_receiver(&m.object, a.span.start)?;
        let mut missing = false;
        let mut enabled = !guarded_key;
        for (_, ancestor) in self.semantic.nodes().ancestors_enumerated(a.node_id.get()) {
            if !self.step() {
                return None;
            }
            let condition = match ancestor.kind() {
                AstKind::IfStatement(i) if contains(i.consequent.span(), a.span) => {
                    Some((&i.test, true))
                }
                AstKind::IfStatement(i)
                    if i.alternate
                        .as_ref()
                        .is_some_and(|s| contains(s.span(), a.span)) =>
                {
                    Some((&i.test, false))
                }
                AstKind::ConditionalExpression(c) if contains(c.consequent.span(), a.span) => {
                    Some((&c.test, true))
                }
                AstKind::ConditionalExpression(c) if contains(c.alternate.span(), a.span) => {
                    Some((&c.test, false))
                }
                AstKind::LogicalExpression(l)
                    if contains(l.right.span(), a.span)
                        && l.operator != LogicalOperator::Coalesce =>
                {
                    Some((&l.left, l.operator == LogicalOperator::And))
                }
                AstKind::Function(_) | AstKind::ArrowFunctionExpression(_) => break,
                _ => None,
            };
            if let Some((condition, truth)) = condition {
                missing |= self.missing_tag(condition, truth, &target, a.span.start, 0);
                enabled |= truth && self.same_key(condition, &m.expression);
            }
        }
        (missing && enabled).then_some(target)
    }
}
