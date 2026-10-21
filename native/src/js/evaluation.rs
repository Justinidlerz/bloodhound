use super::*;

impl<'s, 'a> Analyzer<'s, 'a> {
    pub(super) fn symbol(&self, id: &IdentifierReference<'a>) -> Option<SymbolId> {
        id.reference_id
            .get()
            .and_then(|r| self.scoping.get_reference(r).symbol_id())
    }
    pub(super) fn global_binding(&self, id: SymbolId) -> Option<Target> {
        if self.options.script_mode != "classic"
            || self.scoping.symbol_scope_id(id) != self.scoping.root_scope_id()
        {
            return None;
        }
        let flags = self.scoping.symbol_flags(id);
        if flags.is_import() {
            return None;
        }
        Some(Target::new(
            if flags.is_block_scoped() {
                "global-lexical"
            } else {
                "global-object"
            },
            vec![self.scoping.symbol_name(id).into()],
        ))
    }
    pub(super) fn root(&self, name: &str) -> Value {
        if self.blocked.contains(name) {
            return Value::Unknown;
        }
        let env = self.options.execution_environment.as_str();
        if name == "globalThis"
            || ((name == "window" || name == "self") && env == "window")
            || (name == "self" && env == "worker")
        {
            return Value::Targets(vec![Target::new("global-object", vec![])]);
        }
        // These names are language/host API semantics, never a business allowlist.
        if [
            "Object", "Reflect", "Array", "String", "Number", "Boolean", "Function", "Promise",
            "RegExp", "Date", "Map", "Set", "WeakMap", "WeakSet", "Symbol", "Error", "JSON",
            "Math", "URL",
        ]
        .contains(&name)
            || (name == "document" && env == "window")
        {
            Value::Targets(vec![Target::new("builtin-property", vec![name.into()])])
        } else {
            Value::Targets(vec![Target::new("unbound-reference", vec![name.into()])])
        }
    }
    pub(super) fn ident_value(&self, id: &IdentifierReference<'a>) -> Value {
        if let Some(sym) = self.symbol(id) {
            self.values.get(&sym).cloned().unwrap_or(Value::Unknown)
        } else {
            self.root(id.name.as_str())
        }
    }
    pub(super) fn join(&mut self, a: Value, b: Value, span: Span) -> Value {
        if a == b {
            return a;
        }
        let global_possible = matches!(a, Value::Targets(_) | Value::DynamicTarget)
            || matches!(b, Value::Targets(_) | Value::DynamicTarget);
        match (a, b) {
            (Value::Strings(mut a), Value::Strings(b)) => {
                for v in b {
                    if !a.contains(&v) {
                        a.push(v);
                    }
                }
                if a.len() <= self.options.max_candidates {
                    Value::Strings(a)
                } else {
                    self.gap(span, "candidate-budget", "evaluation");
                    Value::Unknown
                }
            }
            (Value::Targets(mut a), Value::Targets(b)) => {
                for v in b {
                    if !a.contains(&v) {
                        a.push(v);
                    }
                }
                if a.len() <= self.options.max_candidates {
                    Value::Targets(a)
                } else {
                    self.gap(span, "candidate-budget", "evaluation");
                    Value::Unknown
                }
            }
            _ => {
                if global_possible {
                    self.gap(span, "mixed-branch-target", "globals");
                }
                Value::Unknown
            }
        }
    }
    pub(super) fn concat(&mut self, a: Value, b: Value, span: Span) -> Value {
        if let (Value::Number(a), Value::Number(b)) = (&a, &b) {
            return Value::Number(a + b);
        }
        let (a, b) = (a.strings(), b.strings());
        if a.is_empty() || b.is_empty() {
            return Value::Unknown;
        }
        if a.len().saturating_mul(b.len()) > self.options.max_candidates {
            self.gap(span, "candidate-budget", "evaluation");
            return Value::Unknown;
        }
        Value::Strings(
            a.iter()
                .flat_map(|x| b.iter().map(move |y| format!("{x}{y}")))
                .collect(),
        )
    }
    pub(super) fn prop(&mut self, base: Value, keys: Vec<String>, span: Span) -> Value {
        if keys.is_empty() {
            if matches!(base, Value::Targets(_) | Value::DynamicTarget) {
                self.gap(span, "dynamic-property-key", "globals");
                return Value::DynamicTarget;
            }
            return Value::Unknown;
        }
        let mut out = None;
        for key in keys {
            let v = match &base {
                Value::Targets(ts) => Value::Targets(
                    ts.iter()
                        .map(|t| {
                            let mut t = t.clone();
                            t.path_segments.push(key.clone());
                            t
                        })
                        .collect(),
                ),
                Value::Object(m) => m.get(&key).cloned().unwrap_or(Value::Unknown),
                Value::Array(a) => key
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| a.get(i))
                    .cloned()
                    .unwrap_or(Value::Unknown),
                Value::DynamicTarget => Value::DynamicTarget,
                _ => Value::Unknown,
            };
            out = Some(if let Some(prev) = out {
                self.join(prev, v, span)
            } else {
                v
            });
        }
        out.unwrap_or(Value::Unknown)
    }
    pub(super) fn eval(&mut self, e: &Expression<'a>) -> Value {
        if self.eval_depth >= 128 {
            self.gap(e.span(), "evaluation-depth-budget", "evaluation");
            return Value::Unknown;
        }
        self.eval_depth += 1;
        let result = self.eval_inner(e);
        self.eval_depth -= 1;
        result
    }
    pub(super) fn computed(&mut self, base: Value, key: Value, span: Span) -> Value {
        if let Value::SymbolKey(kind, name) = key {
            let mut value = self.prop(
                base,
                vec![serde_json::to_string(&vec![kind.clone(), name]).unwrap()],
                span,
            );
            if let Value::Targets(ts) = &mut value {
                for t in ts {
                    t.key_kind = kind.clone();
                }
            }
            value
        } else {
            self.prop(base, key.strings(), span)
        }
    }
    pub(super) fn eval_inner(&mut self, e: &Expression<'a>) -> Value {
        self.steps += 1;
        if self.steps > self.options.max_steps {
            if self.steps == self.options.max_steps + 1 {
                self.gap(e.span(), "evaluation-step-budget", "evaluation");
            }
            return Value::Unknown;
        }
        match e {
            Expression::StringLiteral(v) => Value::Strings(vec![v.value.to_string()]),
            Expression::NumericLiteral(v) => Value::Number(v.value),
            Expression::BooleanLiteral(v) => Value::Bool(v.value),
            Expression::NullLiteral(_) => Value::Null,
            Expression::Identifier(v) => self.ident_value(v),
            Expression::AssignmentExpression(v) if v.operator.as_str() == "=" => {
                let target = v
                    .left
                    .as_member_expression()
                    .map(|m| self.member_value(m))
                    .unwrap_or(Value::Unknown);
                if matches!(target, Value::Targets(_))
                    && matches!(
                        v.right,
                        Expression::ObjectExpression(_) | Expression::ArrayExpression(_)
                    )
                {
                    target
                } else {
                    self.eval(&v.right)
                }
            }
            Expression::ThisExpression(_) => {
                if self.depth == 0 && self.options.script_mode == "classic" {
                    Value::Targets(vec![Target::new("global-object", vec![])])
                } else {
                    Value::Unknown
                }
            }
            Expression::ParenthesizedExpression(v) => self.eval(&v.expression),
            Expression::StaticMemberExpression(v) => {
                if let Some(value) = self
                    .expression_key(e)
                    .and_then(|k| self.runtime_properties.get(&k))
                    .cloned()
                {
                    return value;
                }
                let b = self.eval(&v.object);
                self.prop(b, vec![v.property.name.to_string()], v.span)
            }
            Expression::ComputedMemberExpression(v) => {
                let b = self.eval(&v.object);
                let k = self.eval(&v.expression);
                self.computed(b, k, v.span)
            }
            Expression::ConditionalExpression(v) => {
                if let Value::Bool(b) = self.eval(&v.test) {
                    return self.eval(if b { &v.consequent } else { &v.alternate });
                }
                let a = self.eval(&v.consequent);
                let b = self.eval(&v.alternate);
                self.join(a, b, v.span)
            }
            Expression::LogicalExpression(v) => {
                let a = self.eval(&v.left);
                let b = self.eval(&v.right);
                self.join(a, b, v.span)
            }
            Expression::UnaryExpression(v) if v.operator.as_str() == "typeof" => {
                let value = self.eval(&v.argument);
                let kind = match &value {
                    Value::Targets(ts) if ts.iter().any(|t| t.namespace == "global-object") => {
                        "object"
                    }
                    Value::Unknown => return Value::Unknown,
                    _ => value.kind(),
                };
                if kind == "unknown" {
                    Value::Unknown
                } else {
                    Value::Strings(vec![kind.into()])
                }
            }
            Expression::UnaryExpression(v) if v.operator.as_str() == "!" => {
                if let Value::Bool(b) = self.eval(&v.argument) {
                    Value::Bool(!b)
                } else {
                    Value::Unknown
                }
            }
            Expression::BinaryExpression(v)
                if ["===", "!==", "==", "!="].contains(&v.operator.as_str()) =>
            {
                let a = self.eval(&v.left);
                let b = self.eval(&v.right);
                if matches!(a, Value::Unknown | Value::Targets(_))
                    || matches!(b, Value::Unknown | Value::Targets(_))
                {
                    Value::Unknown
                } else {
                    Value::Bool(if v.operator.as_str().contains('!') {
                        a != b
                    } else {
                        a == b
                    })
                }
            }
            Expression::BinaryExpression(v) if v.operator.as_str() == "+" => {
                let a = self.eval(&v.left);
                let b = self.eval(&v.right);
                self.concat(a, b, v.span)
            }
            Expression::TemplateLiteral(v) => {
                let mut out = Value::Strings(vec![String::new()]);
                for (i, q) in v.quasis.iter().enumerate() {
                    out = self.concat(
                        out,
                        Value::Strings(vec![
                            q.value
                                .cooked
                                .as_ref()
                                .map(|s| s.to_string())
                                .unwrap_or_default(),
                        ]),
                        v.span,
                    );
                    if let Some(e) = v.expressions.get(i) {
                        let val = self.eval(e);
                        out = self.concat(out, val, v.span);
                    }
                }
                out
            }
            Expression::ObjectExpression(v) => {
                let mut map = BTreeMap::new();
                for p in &v.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = p {
                        if p.method || p.kind != PropertyKind::Init {
                            return Value::Unknown;
                        }
                        let keys = if p.computed {
                            p.key
                                .as_expression()
                                .map(|e| self.eval(e).strings())
                                .unwrap_or_default()
                        } else {
                            p.key
                                .static_name()
                                .map(|s| vec![s.to_string()])
                                .unwrap_or_default()
                        };
                        let val = self.eval(&p.value);
                        for k in keys {
                            map.insert(k, val.clone());
                        }
                    } else {
                        return Value::Unknown;
                    }
                }
                Value::Object(map)
            }
            Expression::ArrayExpression(v) => Value::Array(
                v.elements
                    .iter()
                    .map(|e| {
                        e.as_expression()
                            .map(|e| self.eval(e))
                            .unwrap_or(Value::Unknown)
                    })
                    .collect(),
            ),
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => {
                Value::Typed("function")
            }
            Expression::NewExpression(v) => {
                if matches!(&v.callee,Expression::Identifier(i) if self.symbol(i).is_none() && i.name=="URL")
                {
                    v.arguments
                        .first()
                        .and_then(|a| a.as_expression())
                        .map(|e| self.eval(e))
                        .unwrap_or(Value::Unknown)
                } else {
                    Value::Unknown
                }
            }
            Expression::CallExpression(v) => {
                let key = self.expression_key(&v.callee);
                if let Some((params, expr)) = key.and_then(|k| self.functions.get(&k).cloned()) {
                    let args: Vec<_> = v
                        .arguments
                        .iter()
                        .map(|a| {
                            a.as_expression()
                                .map(|e| self.eval(e))
                                .unwrap_or(Value::Unknown)
                        })
                        .collect();
                    let saved = self.values.clone();
                    for (id, val) in params.into_iter().zip(args) {
                        self.values.insert(id, val);
                    }
                    let result = self.eval(expr);
                    self.values = saved;
                    return result;
                }
                if matches!(&v.callee,Expression::Identifier(i) if self.symbol(i).is_none()&&i.name=="Symbol")
                {
                    return Value::SymbolKey("symbol-local".into(), v.span.start.to_string());
                }
                let callee = self.eval(&v.callee).targets();
                if callee.iter().any(|t| {
                    t.namespace == "builtin-property" && t.path_segments == ["Symbol", "for"]
                }) {
                    if let Some(e) = v.arguments.first().and_then(|a| a.as_expression()) {
                        let names = self.eval(e).strings();
                        if names.len() == 1 {
                            return Value::SymbolKey("symbol-for".into(), names[0].clone());
                        }
                    }
                    return Value::Unknown;
                }
                if callee
                    .iter()
                    .any(|t| t.path_segments == ["document", "createElement"])
                {
                    return Value::Node(
                        v.arguments
                            .first()
                            .and_then(|a| a.as_expression())
                            .map(|e| self.eval(e).strings().join(""))
                            .unwrap_or_default(),
                    );
                }
                Value::Unknown
            }
            _ => Value::Unknown,
        }
    }
}
