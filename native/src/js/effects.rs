use super::*;

impl<'s, 'a> Analyzer<'s, 'a> {
    pub(super) fn access(&mut self, value: Value, span: Span, op: &str, ty: &str, evidence: &str) {
        if self.output.accesses.len() >= self.options.max_steps {
            if self.output.accesses.len() == self.options.max_steps {
                self.gap(span, "access-record-budget", "globals");
            }
            return;
        }
        let ts = value.targets();
        if ts.is_empty() && !matches!(value, Value::DynamicTarget) {
            return;
        }
        self.output.accesses.push(json!({"targets":ts,"start":span.start,"end":span.end,"operation":op,"valueType":ty,"evidence":[evidence,"static-candidate"]}));
    }
    pub(super) fn dep(&mut self, value: Value, span: Span, kind: &str, mode: &str, env: &str) {
        let candidates = value.strings();
        if candidates.is_empty() {
            self.gap(span, "non-finite-dependency", "dependencies");
        }
        self.output.dependencies.push(json!({"specifiers":candidates,"start":span.start,"end":span.end,"kind":kind,"scriptMode":mode,"environment":env}));
    }
    pub(super) fn object_target(&mut self, e: &Expression<'a>) -> Value {
        match e {
            Expression::Identifier(i) => {
                let value = self.ident_value(i);
                if matches!(value, Value::Targets(_) | Value::DynamicTarget) {
                    value
                } else {
                    self.symbol(i)
                        .and_then(|s| self.global_binding(s))
                        .map(|t| Value::Targets(vec![t]))
                        .unwrap_or(Value::Unknown)
                }
            }
            Expression::StaticMemberExpression(m) => {
                let b = self.object_target(&m.object);
                self.prop(b, vec![m.property.name.to_string()], m.span)
            }
            Expression::ComputedMemberExpression(m) => {
                let b = self.object_target(&m.object);
                let key = self.eval(&m.expression);
                self.computed(b, key, m.span)
            }
            _ => self.eval(e),
        }
    }
    pub(super) fn member_value(&mut self, m: &MemberExpression<'a>) -> Value {
        match m {
            MemberExpression::StaticMemberExpression(m) => {
                let b = self.object_target(&m.object);
                self.prop(b, vec![m.property.name.to_string()], m.span)
            }
            MemberExpression::ComputedMemberExpression(m) => {
                let b = self.object_target(&m.object);
                let k = self.eval(&m.expression);
                self.computed(b, k, m.span)
            }
            _ => Value::Unknown,
        }
    }
    pub(super) fn write_target(&mut self, t: &AssignmentTarget<'a>, op: &str, ty: &str) {
        let v = match t {
            AssignmentTarget::AssignmentTargetIdentifier(i) => self
                .symbol(i)
                .and_then(|s| self.global_binding(s))
                .map(|t| Value::Targets(vec![t]))
                .unwrap_or_else(|| {
                    if self.symbol(i).is_none() {
                        Value::Targets(vec![Target::new(
                            "unbound-reference",
                            vec![i.name.to_string()],
                        )])
                    } else {
                        Value::Unknown
                    }
                }),
            _ => t
                .as_member_expression()
                .map(|m| self.member_value(m))
                .unwrap_or(Value::Unknown),
        };
        if op != "write" {
            self.access(v.clone(), t.span(), "read", "unknown", "compound-operation");
        }
        if v.targets().iter().any(|t| {
            t.namespace == "builtin-property"
                && t.path_segments.first().is_some_and(|s| s == "document")
        }) {
            self.output.findings.push(json!({"ruleId":"dom-property-write","kind":"dom","start":t.span().start,"end":t.span().end,"details":{"certainty":"static-candidate","operation":"write"}}));
        }
        self.access(v, t.span(), "write", ty, "binding-and-alias-analysis");
        self.suppressed.insert((t.span().start, t.span().end));
    }
    pub(super) fn inspect_new_expression(&mut self, n: &NewExpression<'a>) {
        if let Expression::Identifier(id) = &n.callee {
            if self.symbol(id).is_none() && ["Worker", "SharedWorker"].contains(&id.name.as_str()) {
                let v = n
                    .arguments
                    .first()
                    .and_then(|a| a.as_expression())
                    .map(|e| self.eval(e))
                    .unwrap_or(Value::Unknown);
                let module=n.arguments.get(1).and_then(|a|a.as_expression()).map(|e|self.eval(e)).is_some_and(|v|matches!(v,Value::Object(m) if m.get("type")==Some(&Value::Strings(vec!["module".into()]))));
                self.dep(
                    v,
                    n.span,
                    "worker",
                    if module { "module" } else { "classic" },
                    "worker",
                );
                let importer = matches!(
                    n.arguments.first().and_then(|a| a.as_expression()),
                    Some(Expression::NewExpression(_))
                );
                let base = if importer {
                    self.options.resource_url.clone()
                } else {
                    self.options.base_url.clone()
                };
                if let Some(base) = base {
                    if let Some(dep) = self.output.dependencies.last_mut() {
                        dep["baseUrl"] = json!(base);
                    }
                } else if !importer {
                    self.gap(n.span, "worker-document-base-unknown", "dependencies");
                }
            }
        }
        walk::walk_new_expression(self, n);
    }
    pub(super) fn inspect_call_expression(&mut self, c: &CallExpression<'a>) {
        let called = match c.callee.get_inner_expression() {
            Expression::FunctionExpression(f) => Some(self.alloc(&**f)),
            Expression::Identifier(i) => self
                .symbol(i)
                .and_then(|s| self.bound_functions.get(&s).copied()),
            _ => None,
        };
        if let Some(f) = called {
            if self.depth >= 64 {
                self.gap(c.span, "wrapper-depth-budget", "globals");
                return;
            }
            let vals: Vec<_> = c
                .arguments
                .iter()
                .map(|a| {
                    a.as_expression()
                        .map(|e| self.eval(e))
                        .unwrap_or(Value::Unknown)
                })
                .collect();
            for a in &c.arguments {
                if !matches!(a.as_expression(), Some(Expression::FunctionExpression(_))) {
                    self.visit_argument(a);
                }
            }
            let saved = self.values.clone();
            let functions = self.bound_functions.clone();
            for ((p, v), a) in f.params.items.iter().zip(vals).zip(c.arguments.iter()) {
                self.bind(&p.pattern, v);
                if let (
                    BindingPattern::BindingIdentifier(p),
                    Some(Expression::FunctionExpression(f)),
                ) = (&p.pattern, a.as_expression())
                {
                    if let Some(id) = p.symbol_id.get() {
                        self.bound_functions.insert(id, self.alloc(&**f));
                    }
                }
            }
            self.depth += 1;
            if let Some(b) = &f.body {
                self.visit_function_body(b);
            }
            self.depth -= 1;
            self.values = saved;
            self.bound_functions = functions;
            self.invalidate_mutable();
            return;
        }
        if let Expression::StaticMemberExpression(m) = &c.callee {
            if let Some(key) = self.expression_key(&m.object) {
                if let Some(arg) = c.arguments.first().and_then(|a| a.as_expression()) {
                    let vals = self.eval(arg).strings();
                    if !vals.is_empty() {
                        let hints = self.load_hints.entry(key).or_default();
                        hints.extend(vals);
                    }
                }
            }
        }
        let callee = self.eval(&c.callee);
        let targets = callee.targets();
        let path = targets
            .first()
            .map(|t| t.path_segments.join("."))
            .unwrap_or_default();
        let args: Vec<_> = c
            .arguments
            .iter()
            .map(|a| {
                a.as_expression()
                    .map(|e| self.eval(e))
                    .unwrap_or(Value::Unknown)
            })
            .collect();
        let operation_target = c
            .arguments
            .first()
            .and_then(|a| a.as_expression())
            .map(|e| self.object_target(e))
            .unwrap_or(Value::Unknown);
        let native = targets
            .first()
            .is_some_and(|t| t.namespace == "builtin-property")
            && !self.blocked.contains(&path);
        if native
            && [
                "Object.defineProperty",
                "Reflect.defineProperty",
                "Reflect.set",
                "Reflect.deleteProperty",
            ]
            .contains(&path.as_str())
        {
            if let (Some(base), Some(key)) = (args.first(), args.get(1)) {
                let v = self.computed(
                    if matches!(operation_target, Value::Targets(_)) {
                        operation_target.clone()
                    } else {
                        base.clone()
                    },
                    key.clone(),
                    c.span,
                );
                self.access(
                    v,
                    c.span,
                    if path.ends_with("deleteProperty") {
                        "delete"
                    } else if path.ends_with(".set") {
                        "write"
                    } else {
                        "define"
                    },
                    args.get(2)
                        .map(|v| {
                            if path.ends_with("defineProperty") {
                                if let Value::Object(m) = v {
                                    m.get("value").map(Value::kind).unwrap_or("unknown")
                                } else {
                                    "unknown"
                                }
                            } else {
                                v.kind()
                            }
                        })
                        .unwrap_or("unknown"),
                    "known-native-operation",
                );
            }
        } else if native && ["Object.assign", "Object.defineProperties"].contains(&path.as_str()) {
            if let Some(base) = args.first() {
                for arg in args.iter().skip(1) {
                    if let Value::Object(map) = arg {
                        for (k, val) in map {
                            let v = self.prop(
                                if matches!(operation_target, Value::Targets(_)) {
                                    operation_target.clone()
                                } else {
                                    base.clone()
                                },
                                vec![k.clone()],
                                c.span,
                            );
                            self.access(
                                v,
                                c.span,
                                if path.ends_with("assign") {
                                    "write"
                                } else {
                                    "define"
                                },
                                if path.ends_with("defineProperties") {
                                    if let Value::Object(m) = val {
                                        m.get("value").map(Value::kind).unwrap_or("unknown")
                                    } else {
                                        "unknown"
                                    }
                                } else {
                                    val.kind()
                                },
                                "known-native-operation",
                            );
                        }
                    } else {
                        self.gap(c.span, "unknown-object-operation-keys", "globals");
                    }
                }
            }
        }
        if native
            && path.starts_with("document.")
            && [
                "appendChild",
                "append",
                "prepend",
                "insertBefore",
                "replaceChild",
                "removeChild",
                "remove",
                "setAttribute",
                "removeAttribute",
                "addEventListener",
                "write",
                "writeln",
                "insertAdjacentHTML",
            ]
            .iter()
            .any(|n| path.ends_with(&format!(".{n}")))
        {
            self.output.findings.push(json!({"ruleId":"dom-operation","kind":"dom","start":c.span.start,"end":c.span.end,"details":{"callee":path,"certainty":"static-candidate"}}));
        }
        if matches!(&c.callee,Expression::Identifier(i) if self.symbol(i).is_none() && i.name=="importScripts")
        {
            for arg in &args {
                self.dep(arg.clone(), c.span, "import-scripts", "classic", "inherit");
            }
        }
        if matches!(&c.callee,Expression::Identifier(i) if self.symbol(i).is_none() && ["eval","Function"].contains(&i.name.as_str()))
        {
            self.gap(c.span, "dynamic-code-execution", "globals");
        }
        // Vite preload helpers are recognized by their function binding and callback import,
        // not the minified helper name. Only literal/finite dependency arrays are accepted.
        if c.arguments
            .first()
            .and_then(|a| a.as_expression())
            .is_some_and(|e| {
                matches!(
                    e,
                    Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)
                )
            })
        {
            let known = matches!(&c.callee,Expression::Identifier(i) if self.symbol(i).is_some_and(|s|self.preload_helpers.contains(&s)));
            if known {
                if let Some(Value::Array(deps)) = args.get(1) {
                    for dep in deps {
                        self.dep(dep.clone(), c.span, "preload", "module", "inherit");
                    }
                } else {
                    self.gap(c.span, "non-finite-preload-array", "dependencies");
                }
            }
        }
        walk::walk_call_expression(self, c);
        if !native {
            if self.values.iter().any(|(id, v)| {
                !self.scoping.symbol_flags(*id).is_const_variable()
                    && matches!(v, Value::Targets(_))
            }) {
                self.gap(c.span, "mutable-alias-after-unknown-call", "globals");
            }
            self.invalidate_mutable();
        }
    }
}
