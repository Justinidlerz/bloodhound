use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::{scope::ScopeFlags, symbol::SymbolId};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    namespace: String,
    path_segments: Vec<String>,
    key_kind: String,
}
impl Target {
    fn new(ns: &str, path: Vec<String>) -> Self {
        Self {
            namespace: ns.into(),
            path_segments: path,
            key_kind: "string".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Unknown,
    Strings(Vec<String>),
    Targets(Vec<Target>),
    Object(BTreeMap<String, Value>),
    Array(Vec<Value>),
    Node(String),
    Typed(&'static str),
    Number(f64),
    Bool(bool),
    Null,
    SymbolKey(String, String),
    DynamicTarget,
}
impl Value {
    fn strings(&self) -> Vec<String> {
        match self {
            Self::Strings(s) => s.clone(),
            Self::Number(n) => vec![n.to_string()],
            Self::Bool(b) => vec![b.to_string()],
            Self::Null => vec!["null".into()],
            _ => vec![],
        }
    }
    fn targets(&self) -> Vec<Target> {
        if let Self::Targets(s) = self {
            s.clone()
        } else {
            vec![]
        }
    }
    fn kind(&self) -> &str {
        match self {
            Self::Strings(_) => "string",
            Self::Object(_) | Self::Node(_) => "object",
            Self::Array(_) => "array",
            Self::Targets(_) | Self::Unknown => "unknown",
            Self::Typed(s) => s,
            Self::Number(_) => "number",
            Self::Bool(_) => "boolean",
            Self::Null => "null",
            Self::SymbolKey(_, _) => "symbol",
            Self::DynamicTarget => "unknown",
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Options {
    script_mode: String,
    execution_environment: String,
    max_steps: usize,
    max_candidates: usize,
    #[serde(default)]
    imported_constants: HashMap<String, Json>,
    #[serde(default)]
    deadline_unix_ms: u64,
    #[serde(default)]
    run_id: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    resource_url: Option<String>,
    #[serde(default)]
    public_path: Option<String>,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Output {
    accesses: Vec<Json>,
    dependencies: Vec<Json>,
    findings: Vec<Json>,
    unresolved: Vec<Json>,
    exports: BTreeMap<String, Json>,
    diagnostics: Vec<String>,
    steps: usize,
}
struct Analyzer<'s, 'a> {
    scoping: &'s Scoping,
    options: Options,
    values: HashMap<SymbolId, Value>,
    functions: HashMap<String, (Vec<SymbolId>, &'a Expression<'a>)>,
    runtime_properties: HashMap<String, Value>,
    export_bindings: HashMap<String, SymbolId>,
    bound_functions: HashMap<SymbolId, &'a Function<'a>>,
    preload_helpers: HashSet<SymbolId>,
    deferred_loads: Vec<(&'a Expression<'a>, Span, HashMap<SymbolId, Value>)>,
    load_hints: HashMap<String, Vec<String>>,
    output: Output,
    depth: usize,
    eval_depth: usize,
    blocked: HashSet<String>,
    suppressed: HashSet<(u32, u32)>,
    steps: usize,
    visits: usize,
}
impl<'s, 'a> Analyzer<'s, 'a> {
    fn gap(&mut self, span: Span, reason: &str, cap: &str) {
        let item =
            json!({"start":span.start,"end":span.end,"reason":reason,"affectedCapabilities":[cap]});
        if !self.output.unresolved.contains(&item) {
            self.output.unresolved.push(item);
        }
    }
    fn symbol(&self, id: &IdentifierReference<'a>) -> Option<SymbolId> {
        id.reference_id
            .get()
            .and_then(|r| self.scoping.get_reference(r).symbol_id())
    }
    fn global_binding(&self, id: SymbolId) -> Option<Target> {
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
    fn root(&self, name: &str) -> Value {
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
    fn ident_value(&self, id: &IdentifierReference<'a>) -> Value {
        if let Some(sym) = self.symbol(id) {
            self.values.get(&sym).cloned().unwrap_or(Value::Unknown)
        } else {
            self.root(id.name.as_str())
        }
    }
    fn join(&mut self, a: Value, b: Value, span: Span) -> Value {
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
    fn concat(&mut self, a: Value, b: Value, span: Span) -> Value {
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
    fn prop(&mut self, base: Value, keys: Vec<String>, span: Span) -> Value {
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
    fn eval(&mut self, e: &Expression<'a>) -> Value {
        if self.eval_depth >= 128 {
            self.gap(e.span(), "evaluation-depth-budget", "evaluation");
            return Value::Unknown;
        }
        self.eval_depth += 1;
        let result = self.eval_inner(e);
        self.eval_depth -= 1;
        result
    }
    fn computed(&mut self, base: Value, key: Value, span: Span) -> Value {
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
    fn eval_inner(&mut self, e: &Expression<'a>) -> Value {
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
    fn expression_key(&self, e: &Expression<'a>) -> Option<String> {
        match e {
            Expression::Identifier(i) => self.symbol(i).map(|s| format!("s{s:?}")),
            Expression::StaticMemberExpression(m) => self
                .expression_key(&m.object)
                .map(|s| format!("{s}.{}", m.property.name)),
            _ => None,
        }
    }
    fn register_function(&mut self, key: String, e: &Expression<'a>) {
        let (params, expr): (&FormalParameters<'a>, Option<&Expression<'a>>) = match e {
            Expression::ArrowFunctionExpression(f) => (
                &f.params,
                if let Some(expr) = f.body.as_expression() {
                    Some(expr)
                } else if let ArrowFunctionBody::FunctionBody(b) = &f.body {
                    if b.statements.len() == 1 {
                        if let Statement::ReturnStatement(r) = &b.statements[0] {
                            r.argument.as_ref()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                },
            ),
            Expression::FunctionExpression(f) => (
                &f.params,
                f.body.as_ref().and_then(|b| {
                    if b.statements.len() == 1 {
                        if let Statement::ReturnStatement(r) = &b.statements[0] {
                            r.argument.as_ref()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }),
            ),
            _ => return,
        };
        if let Some(expr) = expr {
            let ids: Option<Vec<_>> = params
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
            if let Some(ids) = ids {
                self.functions.insert(key, (ids, self.alloc(expr)));
            }
        }
    }
    fn access(&mut self, value: Value, span: Span, op: &str, ty: &str, evidence: &str) {
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
    fn dep(&mut self, value: Value, span: Span, kind: &str, mode: &str, env: &str) {
        let candidates = value.strings();
        if candidates.is_empty() {
            self.gap(span, "non-finite-dependency", "dependencies");
        }
        self.output.dependencies.push(json!({"specifiers":candidates,"start":span.start,"end":span.end,"kind":kind,"scriptMode":mode,"environment":env}));
    }
    fn bind(&mut self, p: &BindingPattern<'a>, value: Value) {
        match p {
            BindingPattern::BindingIdentifier(i) => {
                if let Some(s) = i.symbol_id.get() {
                    self.values.insert(s, value);
                }
            }
            BindingPattern::ObjectPattern(p) => {
                for prop in &p.properties {
                    let keys = prop
                        .key
                        .static_name()
                        .map(|s| vec![s.to_string()])
                        .unwrap_or_default();
                    let val = self.prop(value.clone(), keys, prop.span);
                    self.bind(&prop.value, val);
                }
                if let Some(rest) = &p.rest {
                    self.bind(&rest.argument, Value::Unknown);
                }
            }
            BindingPattern::ArrayPattern(p) => {
                for (i, p) in p.elements.iter().enumerate() {
                    if let Some(p) = p {
                        let val = self.prop(value.clone(), vec![i.to_string()], p.span());
                        self.bind(p, val);
                    }
                }
            }
            BindingPattern::AssignmentPattern(p) => self.bind(&p.left, value),
        }
    }
    fn invalidate_mutable(&mut self) {
        self.values.retain(|id, v| {
            self.scoping.symbol_flags(*id).is_const_variable()
                && !matches!(v, Value::Object(_) | Value::Array(_))
        });
    }
    fn finish_loads(&mut self) {
        let loads = std::mem::take(&mut self.deferred_loads);
        for (expr, span, values) in loads {
            let saved = self.values.clone();
            self.values = values;
            let mut calls = LoadCalls::default();
            calls.visit_expression(expr);
            let mut candidates = Vec::new();
            let mut supported = false;
            for (root, property, arg) in calls.calls {
                let (Some(root), Some(arg)) = (
                    self.scoping.get_reference(root).symbol_id(),
                    self.scoping.get_reference(arg).symbol_id(),
                ) else {
                    continue;
                };
                let key = format!("s{root:?}.{property}");
                if let Some((_, body)) = self.functions.get(&key).cloned() {
                    let mut tables = TableKeys::default();
                    tables.visit_expression(body);
                    let root = key.split('.').next().unwrap_or("");
                    if let Some(hints) = self.load_hints.get(root) {
                        tables.keys.extend(hints.clone());
                    }
                    tables.keys.sort();
                    tables.keys.dedup();
                    if !tables.keys.is_empty() && tables.keys.len() <= self.options.max_candidates {
                        self.values.insert(arg, Value::Strings(tables.keys));
                        supported = true;
                    }
                }
            }
            if supported {
                if let (Some(public_path), Expression::BinaryExpression(b)) =
                    (self.options.public_path.clone(), expr)
                {
                    if let Expression::StaticMemberExpression(prefix) = &b.left {
                        if matches!(b.right, Expression::CallExpression(_)) {
                            if let Some(key) = self.expression_key(&b.left) {
                                self.runtime_properties
                                    .entry(key)
                                    .or_insert(Value::Strings(vec![public_path]));
                            }
                            let _ = prefix;
                        }
                    }
                }
                candidates = self.eval(expr).strings();
            }
            self.values = saved;
            if candidates.is_empty() {
                self.dep(Value::Unknown, span, "dynamic-script", "classic", "inherit");
            } else {
                self.dep(
                    Value::Strings(candidates),
                    span,
                    "webpack-chunk",
                    "classic",
                    "inherit",
                );
            }
        }
    }
    fn object_target(&mut self, e: &Expression<'a>) -> Value {
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
    fn inspect_preload(
        &mut self,
        id: SymbolId,
        body: &FunctionBody<'a>,
        params: &FormalParameters<'a>,
    ) {
        let mut shape = PreloadShape::default();
        shape.visit_function_body(body);
        let first = params.items.first().and_then(|p| {
            if let BindingPattern::BindingIdentifier(i) = &p.pattern {
                i.symbol_id.get()
            } else {
                None
            }
        });
        if first.is_some()
            && shape
                .links
                .iter()
                .any(|r| self.scoping.get_reference(*r).symbol_id().is_none())
            && shape
                .calls
                .iter()
                .any(|r| self.scoping.get_reference(*r).symbol_id() == first)
        {
            self.preload_helpers.insert(id);
        }
    }
    fn member_value(&mut self, m: &MemberExpression<'a>) -> Value {
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
    fn write_target(&mut self, t: &AssignmentTarget<'a>, op: &str, ty: &str) {
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
}
#[derive(Default)]
struct PreloadShape {
    links: Vec<oxc_syntax::reference::ReferenceId>,
    calls: Vec<oxc_syntax::reference::ReferenceId>,
}
impl<'a> Visit<'a> for PreloadShape {
    fn visit_call_expression(&mut self, c: &CallExpression<'a>) {
        if let Expression::Identifier(i) = &c.callee {
            if let Some(r) = i.reference_id.get() {
                self.calls.push(r);
            }
        }
        if let Expression::StaticMemberExpression(m) = &c.callee {
            if m.property.name == "createElement"
                && matches!(c.arguments.first().and_then(|a|a.as_expression()),Some(Expression::StringLiteral(s)) if s.value=="link")
            {
                if let Expression::Identifier(root) = &m.object {
                    if root.name == "document" {
                        if let Some(r) = root.reference_id.get() {
                            self.links.push(r);
                        }
                    }
                }
            }
        }
        walk::walk_call_expression(self, c);
    }
}
#[derive(Default)]
struct LoadCalls {
    calls: Vec<(
        oxc_syntax::reference::ReferenceId,
        String,
        oxc_syntax::reference::ReferenceId,
    )>,
}
impl<'a> Visit<'a> for LoadCalls {
    fn visit_call_expression(&mut self, c: &CallExpression<'a>) {
        if let Expression::StaticMemberExpression(m) = &c.callee {
            if let (Expression::Identifier(root), Some(Expression::Identifier(arg))) = (
                &m.object,
                c.arguments.first().and_then(|a| a.as_expression()),
            ) {
                // Resolve both references through Oxc before associating a runtime binding.
                if let (Some(root), Some(arg)) = (root.reference_id.get(), arg.reference_id.get()) {
                    self.calls.push((root, m.property.name.to_string(), arg));
                }
            }
        }
        walk::walk_call_expression(self, c);
    }
}
#[derive(Default)]
struct TableKeys {
    keys: Vec<String>,
}
impl<'a> Visit<'a> for TableKeys {
    fn visit_computed_member_expression(&mut self, m: &ComputedMemberExpression<'a>) {
        if let Expression::ObjectExpression(o) = &m.object {
            for p in &o.properties {
                if let ObjectPropertyKind::ObjectProperty(p) = p {
                    if let Some(k) = p.key.static_name() {
                        self.keys.push(k.into_owned());
                    }
                }
            }
        }
        walk::walk_computed_member_expression(self, m);
    }
}
impl<'s, 'a> Visit<'a> for Analyzer<'s, 'a> {
    fn visit_expression(&mut self, e: &Expression<'a>) {
        self.visits += 1;
        if self.visits > self.options.max_steps {
            if self.visits == self.options.max_steps + 1 {
                self.gap(e.span(), "ast-visit-budget", "globals");
            }
            return;
        }
        if self.visits % 128 == 0 {
            let expired = self.options.deadline_unix_ms > 0
                && crate::model::now() >= self.options.deadline_unix_ms;
            let cancelled = !self.options.run_id.is_empty()
                && crate::cancellation(&self.options.run_id)
                    .load(std::sync::atomic::Ordering::Relaxed);
            if expired || cancelled {
                self.gap(
                    e.span(),
                    if cancelled {
                        "cancelled"
                    } else {
                        "analysis-time-budget"
                    },
                    "globals",
                );
                self.visits = self.options.max_steps + 1;
                return;
            }
        }
        walk::walk_expression(self, e);
    }

    fn visit_variable_declarator(&mut self, v: &VariableDeclarator<'a>) {
        if let Some(init) = &v.init {
            self.visit_expression(init);
        }
        let value = v
            .init
            .as_ref()
            .map(|e| self.eval(e))
            .unwrap_or(Value::Unknown);
        if let BindingPattern::BindingIdentifier(id) = &v.id {
            if let Some(s) = id.symbol_id.get() {
                if let Some(init) = &v.init {
                    self.register_function(format!("s{s:?}"), init);
                    match init {
                        Expression::FunctionExpression(f) => {
                            if let Some(b) = &f.body {
                                self.inspect_preload(s, b, &f.params);
                            }
                        }
                        Expression::ArrowFunctionExpression(f) => {
                            if let ArrowFunctionBody::FunctionBody(b) = &f.body {
                                self.inspect_preload(s, b, &f.params);
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(t) = self.global_binding(s) {
                    let ty = v.init.as_ref().map(value_type).unwrap_or("undefined");
                    self.access(
                        Value::Targets(vec![t]),
                        id.span,
                        "declare",
                        ty,
                        "classic-top-level-binding",
                    );
                }
            }
        }
        if !matches!(v.id, BindingPattern::BindingIdentifier(_)) {
            self.visit_binding_pattern(&v.id);
        }
        self.bind(&v.id, value);
    }
    fn visit_binding_identifier(&mut self, i: &BindingIdentifier<'a>) {
        if let Some(t) = i.symbol_id.get().and_then(|s| self.global_binding(s)) {
            self.access(
                Value::Targets(vec![t]),
                i.span,
                "declare",
                "unknown",
                "top-level-binding",
            );
        }
    }
    fn visit_identifier_reference(&mut self, i: &IdentifierReference<'a>) {
        if self.suppressed.contains(&(i.span.start, i.span.end)) {
            return;
        }
        let v = if let Some(s) = self.symbol(i) {
            self.global_binding(s)
                .map(|t| Value::Targets(vec![t]))
                .unwrap_or(Value::Unknown)
        } else {
            self.root(i.name.as_str())
        };
        self.access(v, i.span, "read", "unknown", "semantic-reference");
    }
    fn visit_static_member_expression(&mut self, m: &StaticMemberExpression<'a>) {
        if !self.suppressed.contains(&(m.span.start, m.span.end)) {
            let b = self.object_target(&m.object);
            let v = self.prop(b, vec![m.property.name.to_string()], m.span);
            self.access(v, m.span, "read", "unknown", "member-and-alias-analysis");
        }
        self.visit_expression(&m.object);
    }
    fn visit_computed_member_expression(&mut self, m: &ComputedMemberExpression<'a>) {
        if !self.suppressed.contains(&(m.span.start, m.span.end)) {
            let b = self.object_target(&m.object);
            let k = self.eval(&m.expression);
            let v = self.computed(b, k, m.span);
            self.access(v, m.span, "read", "unknown", "finite-property-evaluation");
        }
        self.visit_expression(&m.object);
        self.visit_expression(&m.expression);
    }
    fn visit_assignment_expression(&mut self, a: &AssignmentExpression<'a>) {
        self.visit_expression(&a.right);
        let value = self.eval(&a.right);
        self.write_target(
            &a.left,
            if a.operator.as_str() == "=" {
                "write"
            } else {
                "compound"
            },
            value_type(&a.right),
        );
        self.visit_assignment_target(&a.left);
        match &a.left {
            AssignmentTarget::AssignmentTargetIdentifier(i) => {
                if let Some(s) = self.symbol(i) {
                    self.values.insert(
                        s,
                        if a.operator.as_str() == "=" {
                            value
                        } else {
                            Value::Unknown
                        },
                    );
                    self.functions.remove(&format!("s{s:?}"));
                    self.runtime_properties
                        .retain(|k, _| !k.starts_with(&format!("s{s:?}.")));
                } else {
                    self.blocked.insert(i.name.to_string());
                }
            }
            AssignmentTarget::StaticMemberExpression(m) => {
                if let Some(key) = self.expression_key(&m.object) {
                    let full = format!("{key}.{}", m.property.name);
                    self.register_function(full.clone(), &a.right);
                    if !matches!(value, Value::Unknown | Value::Typed("function")) {
                        self.runtime_properties.insert(full, value.clone());
                    }
                }
                if let Expression::Identifier(id) = &m.object {
                    if let Some(s) = self.symbol(id) {
                        let prop = m.property.name.to_string();
                        if let Some(Value::Node(tag)) = self.values.get(&s).cloned() {
                            if (tag == "script"
                                && ["text", "textContent", "innerHTML"].contains(&prop.as_str()))
                                || (tag == "iframe" && prop == "srcdoc")
                            {
                                let candidates = value.strings();
                                if candidates.is_empty() {
                                    self.gap(a.span, "non-finite-inline-code", "dependencies");
                                } else {
                                    for content in candidates {
                                        self.output.dependencies.push(json!({"start":a.span.start,"end":a.span.end,"kind":if tag=="iframe"{"iframe"}else{"script"},"scriptMode":"classic","environment":if tag=="iframe"{"window"}else{"inherit"},"specifiers":[],"content":content,"condition":"node-insertion-not-proven","sourceLocationApproximate":true}));
                                    }
                                }
                            }
                            if (tag == "script" && prop == "src")
                                || (tag == "link" && prop == "href")
                                || (tag == "iframe" && prop == "src")
                            {
                                if tag == "script" && value.strings().is_empty() {
                                    self.deferred_loads.push((
                                        self.alloc(&a.right),
                                        a.span,
                                        self.values.clone(),
                                    ));
                                } else {
                                    self.dep(
                                        value.clone(),
                                        a.span,
                                        if tag == "iframe" {
                                            "iframe"
                                        } else if tag == "link" {
                                            "style"
                                        } else {
                                            "script"
                                        },
                                        "classic",
                                        if tag == "iframe" { "window" } else { "inherit" },
                                    );
                                }
                            }
                        }
                        let entry = self
                            .values
                            .entry(s)
                            .or_insert_with(|| Value::Object(BTreeMap::new()));
                        if let Value::Object(map) = entry {
                            map.insert(prop, value);
                        }
                    }
                }
                let ts = self.eval(&m.object).targets();
                for t in ts {
                    if t.namespace == "builtin-property" {
                        self.blocked.insert(t.path_segments.join("."));
                    }
                }
            }
            AssignmentTarget::ComputedMemberExpression(m) => {
                let keys = self.eval(&m.expression).strings();
                if let Expression::Identifier(i) = &m.object {
                    if let Some(id) = self.symbol(i) {
                        if keys.len() == 1 {
                            if let Some(Value::Object(map)) = self.values.get_mut(&id) {
                                map.insert(keys[0].clone(), value);
                            }
                        } else {
                            self.values.remove(&id);
                        }
                    }
                }
            }
            _ => {
                self.gap(
                    a.left.span(),
                    "complex-assignment-propagation",
                    "evaluation",
                );
                self.invalidate_mutable();
            }
        }
    }
    fn visit_update_expression(&mut self, u: &UpdateExpression<'a>) {
        let v = match &u.argument {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(i) => {
                if let Some(s) = self.symbol(i) {
                    self.values.remove(&s);
                    self.global_binding(s)
                        .map(|t| Value::Targets(vec![t]))
                        .unwrap_or(Value::Unknown)
                } else {
                    Value::Targets(vec![Target::new(
                        "unbound-reference",
                        vec![i.name.to_string()],
                    )])
                }
            }
            _ => u
                .argument
                .as_member_expression()
                .map(|m| self.member_value(m))
                .unwrap_or(Value::Unknown),
        };
        self.access(v.clone(), u.span, "read", "unknown", "update");
        self.access(v, u.span, "write", "number", "update");
        self.suppressed
            .insert((u.argument.span().start, u.argument.span().end));
        walk::walk_update_expression(self, u);
    }
    fn visit_unary_expression(&mut self, u: &UnaryExpression<'a>) {
        if u.operator.as_str() == "delete" {
            let v = self.object_target(&u.argument);
            self.access(v, u.span, "delete", "undefined", "delete-expression");
            self.suppressed
                .insert((u.argument.span().start, u.argument.span().end));
        }
        walk::walk_unary_expression(self, u);
    }
    fn visit_function(&mut self, f: &Function<'a>, flags: ScopeFlags) {
        if let (Some(id), Some(body)) = (
            f.id.as_ref().and_then(|i| i.symbol_id.get()),
            f.body.as_ref(),
        ) {
            self.inspect_preload(id, body, &f.params);
        }
        let saved = self.values.clone();
        if saved.iter().any(|(id, v)| {
            !self.scoping.symbol_flags(*id).is_const_variable() && matches!(v, Value::Targets(_))
        }) {
            self.gap(f.span, "mutable-captured-alias", "globals");
        }
        self.invalidate_mutable();
        self.depth += 1;
        walk::walk_function(self, f, flags);
        self.depth -= 1;
        self.values = saved;
    }
    fn visit_arrow_function_expression(&mut self, f: &ArrowFunctionExpression<'a>) {
        let saved = self.values.clone();
        self.invalidate_mutable();
        self.depth += 1;
        walk::walk_arrow_function_expression(self, f);
        self.depth -= 1;
        self.values = saved;
    }
    fn visit_if_statement(&mut self, s: &IfStatement<'a>) {
        self.visit_expression(&s.test);
        let saved = self.values.clone();
        self.visit_statement(&s.consequent);
        let left = self.values.clone();
        self.values = saved;
        if let Some(alt) = &s.alternate {
            self.visit_statement(alt);
        }
        let right = self.values.clone();
        for (id, a) in left {
            let b = right.get(&id).cloned().unwrap_or(Value::Unknown);
            let val = self.join(a, b, s.span);
            self.values.insert(id, val);
        }
    }
    fn visit_conditional_expression(&mut self, e: &ConditionalExpression<'a>) {
        self.visit_expression(&e.test);
        let saved = self.values.clone();
        self.visit_expression(&e.consequent);
        let left = self.values.clone();
        self.values = saved;
        self.visit_expression(&e.alternate);
        let right = self.values.clone();
        for (id, a) in left {
            let b = right.get(&id).cloned().unwrap_or(Value::Unknown);
            let val = self.join(a, b, e.span);
            self.values.insert(id, val);
        }
    }
    fn visit_for_statement(&mut self, s: &ForStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_for_statement(self, s);
        self.invalidate_mutable();
    }
    fn visit_for_in_statement(&mut self, s: &ForInStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_for_in_statement(self, s);
        self.invalidate_mutable();
    }
    fn visit_for_of_statement(&mut self, s: &ForOfStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_for_of_statement(self, s);
        self.invalidate_mutable();
    }
    fn visit_while_statement(&mut self, s: &WhileStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_while_statement(self, s);
        self.invalidate_mutable();
    }
    fn visit_do_while_statement(&mut self, s: &DoWhileStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_do_while_statement(self, s);
        self.invalidate_mutable();
    }
    fn visit_switch_statement(&mut self, s: &SwitchStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_switch_statement(self, s);
        self.invalidate_mutable();
        self.gap(s.span, "switch-flow-not-propagated", "evaluation");
    }
    fn visit_try_statement(&mut self, s: &TryStatement<'a>) {
        self.invalidate_mutable();
        walk::walk_try_statement(self, s);
        self.invalidate_mutable();
        self.gap(s.span, "exception-flow-not-propagated", "evaluation");
    }
    fn visit_with_statement(&mut self, s: &WithStatement<'a>) {
        self.gap(s.span, "with-dynamic-scope", "globals");
    }
    fn visit_import_declaration(&mut self, i: &ImportDeclaration<'a>) {
        self.dep(
            Value::Strings(vec![i.source.value.to_string()]),
            i.span,
            "import",
            "module",
            "inherit",
        );
        if let Some(specs) = &i.specifiers {
            for spec in specs {
                if let ImportDeclarationSpecifier::ImportSpecifier(s) = spec {
                    let name = s.imported.name().to_string();
                    let key = format!("{}#{name}", i.source.value);
                    if let Some(value) = self.options.imported_constants.get(&key) {
                        if let Some(id) = s.local.symbol_id.get() {
                            self.values.insert(id, json_value(value));
                        }
                    }
                }
            }
        }
        if let Some(specifiers) = &i.specifiers {
            for spec in specifiers {
                match spec {
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                        let key = format!("{}#default", i.source.value);
                        if let (Some(id), Some(v)) = (
                            s.local.symbol_id.get(),
                            self.options.imported_constants.get(&key),
                        ) {
                            self.values.insert(id, json_value(v));
                        }
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                        let prefix = format!("{}#", i.source.value);
                        let map = self
                            .options
                            .imported_constants
                            .iter()
                            .filter_map(|(k, v)| {
                                k.strip_prefix(&prefix)
                                    .map(|name| (name.to_string(), json_value(v)))
                            })
                            .collect();
                        if let Some(id) = s.local.symbol_id.get() {
                            self.values.insert(id, Value::Object(map));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    fn visit_export_named_declaration(&mut self, e: &ExportNamedDeclaration<'a>) {
        walk::walk_export_named_declaration(self, e);
        for spec in &e.specifiers {
            if let ModuleExportName::IdentifierReference(i) = &spec.local {
                if let Some(id) = self.symbol(i) {
                    self.export_bindings
                        .insert(spec.exported.name().to_string(), id);
                }
                let val = self.ident_value(i);
                if let Some(v) = to_json(&val) {
                    self.output
                        .exports
                        .insert(spec.exported.name().to_string(), v);
                }
            }
        }
    }
    fn visit_export_from_declaration(&mut self, e: &ExportFromDeclaration<'a>) {
        self.dep(
            Value::Strings(vec![e.source.value.to_string()]),
            e.span,
            "export",
            "module",
            "inherit",
        );
        for spec in &e.specifiers {
            let key = format!("{}#{}", e.source.value, spec.local.name());
            if let Some(value) = self.options.imported_constants.get(&key) {
                self.output
                    .exports
                    .insert(spec.exported.name().to_string(), value.clone());
            }
        }
    }
    fn visit_export_declaration(&mut self, e: &ExportDeclaration<'a>) {
        walk::walk_export_declaration(self, e);
        if let Declaration::VariableDeclaration(v) = &e.declaration {
            for d in &v.declarations {
                if let BindingPattern::BindingIdentifier(i) = &d.id {
                    if let Some(id) = i.symbol_id.get() {
                        self.export_bindings.insert(i.name.to_string(), id);
                    }
                    if let Some(v) = i
                        .symbol_id
                        .get()
                        .and_then(|s| self.values.get(&s))
                        .and_then(to_json)
                    {
                        self.output.exports.insert(i.name.to_string(), v);
                    }
                }
            }
        }
    }
    fn visit_export_default_declaration(&mut self, e: &ExportDefaultDeclaration<'a>) {
        walk::walk_export_default_declaration(self, e);
        if let Some(expr) = e.declaration.as_expression() {
            let value = self.eval(expr);
            if let Some(v) = if matches!(
                value,
                Value::Strings(_) | Value::Number(_) | Value::Bool(_) | Value::Null
            ) {
                to_json(&value)
            } else {
                None
            } {
                self.output.exports.insert("default".into(), v);
            }
        }
    }
    fn visit_export_all_declaration(&mut self, e: &ExportAllDeclaration<'a>) {
        self.dep(
            Value::Strings(vec![e.source.value.to_string()]),
            e.span,
            "export",
            "module",
            "inherit",
        );
        let prefix = format!("{}#", e.source.value);
        let mut map = serde_json::Map::new();
        for (key, value) in &self.options.imported_constants {
            if let Some(name) = key.strip_prefix(&prefix) {
                if name != "default" {
                    map.insert(name.into(), value.clone());
                }
            }
        }
        if let Some(name) = &e.exported {
            self.output
                .exports
                .insert(name.name().to_string(), Json::Object(map));
        } else {
            self.output.exports.extend(map);
        }
    }
    fn visit_import_expression(&mut self, i: &ImportExpression<'a>) {
        let v = self.eval(&i.source);
        self.dep(v, i.span, "dynamic-import", "module", "inherit");
        walk::walk_import_expression(self, i);
    }
    fn visit_new_expression(&mut self, n: &NewExpression<'a>) {
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
    fn visit_call_expression(&mut self, c: &CallExpression<'a>) {
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
fn value_type(e: &Expression<'_>) -> &'static str {
    match e {
        Expression::NumericLiteral(_) => "number",
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => "string",
        Expression::BooleanLiteral(_) => "boolean",
        Expression::ObjectExpression(_) => "object",
        Expression::ArrayExpression(_) => "array",
        Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => "function",
        Expression::NullLiteral(_) => "null",
        _ => "unknown",
    }
}
fn to_json(v: &Value) -> Option<Json> {
    match v {
        Value::Number(n) => Some(json!(n)),
        Value::Bool(b) => Some(json!(b)),
        Value::Null => Some(Json::Null),
        Value::Strings(s) if s.len() == 1 => Some(json!(s[0])),
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| to_json(v).map(|v| (k.clone(), v)))
            .collect::<Option<serde_json::Map<_, _>>>()
            .map(Json::Object),
        Value::Array(a) => a
            .iter()
            .map(to_json)
            .collect::<Option<Vec<_>>>()
            .map(Json::Array),
        _ => None,
    }
}
fn json_value(v: &Json) -> Value {
    match v {
        Json::String(s) => Value::Strings(vec![s.clone()]),
        Json::Number(n) => n.as_f64().map(Value::Number).unwrap_or(Value::Unknown),
        Json::Bool(b) => Value::Bool(*b),
        Json::Null => Value::Null,
        Json::Object(m) => {
            Value::Object(m.iter().map(|(k, v)| (k.clone(), json_value(v))).collect())
        }
        Json::Array(a) => Value::Array(a.iter().map(json_value).collect()),
    }
}
pub fn engine_version() -> String {
    "oxc-0.154.0".into()
}
pub fn analyze(source: String, options_json: String) -> napi::Result<String> {
    let options: Options =
        serde_json::from_str(&options_json).map_err(|e| napi::Error::from_reason(e.to_string()))?;
    let allocator = Allocator::default();
    let source_type = if options.script_mode == "module" {
        SourceType::mjs()
    } else {
        if options.script_mode == "unknown" {
            SourceType::unambiguous()
        } else {
            SourceType::script()
        }
    };
    let parsed = Parser::new(&allocator, &source, source_type).parse();
    let mut output = Output::default();
    output.diagnostics = parsed
        .diagnostics
        .iter()
        .map(|d| format!("{d:?}"))
        .collect();
    if !output.diagnostics.is_empty() {
        output.unresolved.push(json!({"start":0,"end":source.len(),"reason":"parse-error","affectedCapabilities":["globals","dependencies"]}));
        return Ok(serde_json::to_string(&output).unwrap());
    }
    let semantic = SemanticBuilder::new_compiler()
        .with_check_syntax_error(true)
        .build(&parsed.program);
    output
        .diagnostics
        .extend(semantic.diagnostics.iter().map(|d| format!("{d:?}")));
    let mut analyzer = Analyzer {
        scoping: semantic.semantic.scoping(),
        options,
        values: HashMap::new(),
        functions: HashMap::new(),
        runtime_properties: HashMap::new(),
        bound_functions: HashMap::new(),
        export_bindings: HashMap::new(),
        preload_helpers: HashSet::new(),
        deferred_loads: vec![],
        load_hints: HashMap::new(),
        output,
        depth: 0,
        eval_depth: 0,
        blocked: HashSet::new(),
        suppressed: HashSet::new(),
        steps: 0,
        visits: 0,
    };
    if analyzer.options.script_mode == "unknown" {
        analyzer.gap(parsed.program.span, "missing-script-mode", "globals");
    }
    if analyzer.options.execution_environment == "unknown" {
        analyzer.gap(
            parsed.program.span,
            "missing-execution-environment",
            "globals",
        );
    }
    if !analyzer.output.diagnostics.is_empty() {
        analyzer.gap(parsed.program.span, "semantic-error", "globals");
    }
    analyzer.visit_program(&parsed.program);
    analyzer.finish_loads();
    for (name, id) in analyzer.export_bindings.clone() {
        let value = analyzer
            .values
            .get(&id)
            .filter(|_| analyzer.scoping.symbol_flags(id).is_const_variable())
            .and_then(|v| {
                if matches!(
                    v,
                    Value::Strings(_) | Value::Number(_) | Value::Bool(_) | Value::Null
                ) {
                    to_json(v)
                } else {
                    None
                }
            });
        if let Some(value) = value {
            analyzer.output.exports.insert(name, value);
        } else {
            analyzer.output.exports.remove(&name);
        }
    }
    analyzer.output.steps = analyzer.steps;
    Ok(serde_json::to_string(&analyzer.output).unwrap())
}
