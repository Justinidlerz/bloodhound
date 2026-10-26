//! Abstract expression evaluation.
//!
//! `eval` answers one question for an expression: "what global thing could
//! this refer to, or what string could it be?" It handles identifiers (via
//! the symbol table), member chains, literals, templates, concatenation,
//! conditionals, object/array literals, `document.createElement`, and
//! `new URL(x, import.meta.url)`. It is deliberately shallow: no calls are
//! followed except the handful of builtins listed here.

use super::Analyzer;
use super::builtins;
use super::values::{Node, Root, Target, Value};
use oxc_ast::ast::*;
use oxc_syntax::symbol::SymbolId;

impl<'s, 'c> Analyzer<'s, 'c> {
    /// Symbol bound to an identifier reference, if it resolves to a binding
    /// in this file.
    pub(super) fn symbol_of(&self, ident: &IdentifierReference<'_>) -> Option<SymbolId> {
        ident
            .reference_id
            .get()
            .and_then(|r| self.scoping.get_reference(r).symbol_id())
    }

    /// Whether a binding may be reassigned after initialization.
    pub(super) fn is_mutable(&self, symbol: SymbolId) -> bool {
        !self.scoping.symbol_flags(symbol).is_const_variable()
            && self.scoping.symbol_is_mutated(symbol)
    }

    /// Value of an identifier: a tracked binding, or a classified global.
    pub(super) fn ident_value(&mut self, ident: &IdentifierReference<'_>) -> Value {
        match self.symbol_of(ident) {
            Some(symbol) => {
                let value = self.values.get(&symbol).cloned().unwrap_or(Value::Unknown);
                if value.is_tracked() && self.is_mutable(symbol) {
                    self.uncertain = true;
                }
                value
            }
            None => match builtins::classify(&ident.name) {
                Some(root) => Value::Targets(vec![Target::new(root)]),
                None => Value::Unknown,
            },
        }
    }

    /// Evaluates an expression to an abstract value.
    pub(super) fn eval(&mut self, expr: &Expression<'_>) -> Value {
        self.eval_depth(expr, 0)
    }

    fn eval_depth(&mut self, expr: &Expression<'_>, depth: usize) -> Value {
        if depth > 64 {
            return Value::Unknown;
        }
        let max = self.options.max_candidates;
        match expr.get_inner_expression() {
            Expression::Identifier(id) => self.ident_value(id),
            Expression::ThisExpression(_) => {
                self.this_stack.last().cloned().unwrap_or(Value::Unknown)
            }
            Expression::StringLiteral(s) => Value::Strings(vec![s.value.to_string()]),
            Expression::NumericLiteral(n) => Value::Number(n.value),
            Expression::BooleanLiteral(b) => Value::Bool(b.value),
            Expression::NullLiteral(_) => Value::Null,
            Expression::TemplateLiteral(t) => {
                let mut out = Value::Strings(vec![String::new()]);
                for (i, quasi) in t.quasis.iter().enumerate() {
                    let text = quasi
                        .value
                        .cooked
                        .as_ref()
                        .map(|c| c.to_string())
                        .unwrap_or_default();
                    out = concat(out, Value::Strings(vec![text]), max);
                    if let Some(e) = t.expressions.get(i) {
                        let v = self.eval_depth(e, depth + 1);
                        out = concat(out, v, max);
                    }
                }
                out
            }
            Expression::BinaryExpression(b) if b.operator.as_str() == "+" => {
                let l = self.eval_depth(&b.left, depth + 1);
                let r = self.eval_depth(&b.right, depth + 1);
                concat(l, r, max)
            }
            Expression::StaticMemberExpression(m) => {
                let object = self.eval_depth(&m.object, depth + 1);
                self.member(object, m.property.name.as_str())
            }
            Expression::ComputedMemberExpression(m) => {
                let object = self.eval_depth(&m.object, depth + 1);
                let key = self.eval_depth(&m.expression, depth + 1);
                let keys = key.strings();
                if keys.len() == 1 {
                    self.member(object, &keys[0])
                } else {
                    // Dynamic key on a tracked target: still a target, with an
                    // unknown segment so the finding says `window[*]`.
                    match object {
                        Value::Targets(ts) => {
                            Value::Targets(ts.iter().map(|t| t.child("*")).collect())
                        }
                        _ => Value::Unknown,
                    }
                }
            }
            Expression::ChainExpression(c) => match &c.expression {
                ChainElement::StaticMemberExpression(m) => {
                    let object = self.eval_depth(&m.object, depth + 1);
                    self.member(object, m.property.name.as_str())
                }
                ChainElement::ComputedMemberExpression(m) => {
                    let object = self.eval_depth(&m.object, depth + 1);
                    let keys = self.eval_depth(&m.expression, depth + 1).strings();
                    if keys.len() == 1 {
                        self.member(object, &keys[0])
                    } else {
                        Value::Unknown
                    }
                }
                _ => Value::Unknown,
            },
            Expression::ParenthesizedExpression(p) => self.eval_depth(&p.expression, depth + 1),
            Expression::SequenceExpression(s) => s
                .expressions
                .last()
                .map(|e| self.eval_depth(e, depth + 1))
                .unwrap_or(Value::Unknown),
            Expression::AssignmentExpression(a) if a.operator.as_str() == "=" => {
                self.eval_depth(&a.right, depth + 1)
            }
            Expression::ConditionalExpression(c) => {
                let test = self.eval_depth(&c.test, depth + 1);
                match test {
                    Value::Bool(true) => self.eval_depth(&c.consequent, depth + 1),
                    Value::Bool(false) => self.eval_depth(&c.alternate, depth + 1),
                    _ => {
                        let a = self.eval_depth(&c.consequent, depth + 1);
                        let b = self.eval_depth(&c.alternate, depth + 1);
                        self.join_branches(a, b)
                    }
                }
            }
            Expression::LogicalExpression(l) => {
                let a = self.eval_depth(&l.left, depth + 1);
                let b = self.eval_depth(&l.right, depth + 1);
                // `window.foo || {}` keeps the target; `a ?? b` similar.
                match (&a, &b) {
                    (Value::Targets(_), _) => a,
                    (_, Value::Targets(_)) => b,
                    _ => self.join_branches(a, b),
                }
            }
            Expression::UnaryExpression(u) => match u.operator {
                UnaryOperator::Typeof => {
                    let v = self.eval_depth(&u.argument, depth + 1);
                    match v {
                        Value::Targets(_) | Value::Object(_) | Value::Node(_) | Value::Null => {
                            Value::Strings(vec!["object".into()])
                        }
                        Value::Function => Value::Strings(vec!["function".into()]),
                        other => other
                            .type_name()
                            .map(|t| Value::Strings(vec![t.into()]))
                            .unwrap_or(Value::Unknown),
                    }
                }
                // `!1`/`!0` are the minified spellings of false/true, so
                // negation must understand literal truthiness, not only booleans.
                UnaryOperator::LogicalNot => match self.eval_depth(&u.argument, depth + 1) {
                    Value::Bool(b) => Value::Bool(!b),
                    Value::Number(n) => Value::Bool(n == 0.0 || n.is_nan()),
                    Value::Strings(s) if !s.is_empty() && s.iter().all(|x| x.is_empty()) => {
                        Value::Bool(true)
                    }
                    Value::Strings(s) if !s.is_empty() && s.iter().all(|x| !x.is_empty()) => {
                        Value::Bool(false)
                    }
                    Value::Null | Value::Undefined => Value::Bool(true),
                    Value::Targets(_)
                    | Value::Node(_)
                    | Value::Object(_)
                    | Value::Array(_)
                    | Value::Function => Value::Bool(false),
                    _ => Value::Unknown,
                },
                UnaryOperator::Void => Value::Undefined,
                _ => Value::Unknown,
            },
            Expression::ObjectExpression(o) => {
                let mut map = std::collections::BTreeMap::new();
                for prop in &o.properties {
                    match prop {
                        ObjectPropertyKind::ObjectProperty(p) => {
                            let key = if p.computed {
                                p.key
                                    .as_expression()
                                    .map(|e| self.eval_depth(e, depth + 1).strings())
                                    .unwrap_or_default()
                            } else {
                                p.key
                                    .static_name()
                                    .map(|s| vec![s.to_string()])
                                    .unwrap_or_default()
                            };
                            let value = if p.method || p.kind != PropertyKind::Init {
                                Value::Function
                            } else {
                                self.eval_depth(&p.value, depth + 1)
                            };
                            for k in key {
                                map.insert(k, value.clone());
                            }
                        }
                        ObjectPropertyKind::SpreadProperty(_) => return Value::Unknown,
                    }
                }
                Value::Object(map)
            }
            Expression::ArrayExpression(a) => Value::Array(
                a.elements
                    .iter()
                    .map(|e| {
                        e.as_expression()
                            .map(|e| self.eval_depth(e, depth + 1))
                            .unwrap_or(Value::Unknown)
                    })
                    .collect(),
            ),
            Expression::FunctionExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::ClassExpression(_) => Value::Function,
            Expression::NewExpression(n) => self.eval_new(n, depth),
            Expression::CallExpression(c) => self.eval_call(c, depth),
            Expression::ImportMeta(_) => {
                let mut map = std::collections::BTreeMap::new();
                map.insert(
                    "url".into(),
                    Value::Strings(vec![self.options.resource_url.clone()]),
                );
                Value::Object(map)
            }
            _ => Value::Unknown,
        }
    }

    /// Joins two branch values and marks the result uncertain when they
    /// disagree about a target.
    fn join_branches(&mut self, a: Value, b: Value) -> Value {
        let both_targets = matches!(a, Value::Targets(_)) && matches!(b, Value::Targets(_));
        let joined = a.join(b, self.options.max_candidates);
        if both_targets && matches!(joined, Value::Targets(ref t) if t.len() > 1) {
            self.uncertain = true;
        }
        joined
    }

    /// Property access that also re-roots `window.<host api>` so that
    /// `window.fetch` and `fetch` are the same target.
    pub(super) fn member(&self, object: Value, key: &str) -> Value {
        match object {
            Value::Targets(ts) => Value::Targets(
                ts.into_iter()
                    .map(|t| {
                        if t.root == Root::Global && t.path.is_empty() {
                            match builtins::reclassify_global_child(key) {
                                Some(Root::Global) => Target::new(Root::Global),
                                Some(root) => Target::new(root),
                                None if key == "document" => Target::new(Root::Document),
                                None => t.child(key),
                            }
                        } else {
                            t.child(key)
                        }
                    })
                    .collect(),
            ),
            other => other.property(key),
        }
    }

    fn eval_new(&mut self, n: &NewExpression<'_>, depth: usize) -> Value {
        let callee = self.eval_depth(&n.callee, depth + 1);
        let targets = callee.targets();
        let Some(t) = targets.first() else {
            return Value::Unknown;
        };
        if t.path.is_empty() {
            match &t.root {
                // `new URL(relative, import.meta.url)` resolves to a string.
                Root::Host("URL") => {
                    let href = n
                        .arguments
                        .first()
                        .and_then(|a| a.as_expression())
                        .map(|e| self.eval_depth(e, depth + 1));
                    let base = n
                        .arguments
                        .get(1)
                        .and_then(|a| a.as_expression())
                        .map(|e| self.eval_depth(e, depth + 1));
                    let hrefs = href.map(|v| v.strings()).unwrap_or_default();
                    let bases = base.map(|v| v.strings()).unwrap_or_default();
                    if hrefs.is_empty() {
                        return Value::Unknown;
                    }
                    if bases.is_empty() {
                        return Value::Strings(hrefs);
                    }
                    let mut out = vec![];
                    for b in &bases {
                        for h in &hrefs {
                            if let Ok(u) = crate::resolve::join(b, h) {
                                if !out.contains(&u) {
                                    out.push(u);
                                }
                            }
                        }
                    }
                    if out.is_empty() {
                        Value::Unknown
                    } else {
                        Value::Strings(out)
                    }
                }
                Root::Host("Image") => Value::Node(Node::new(n.span.start, "img")),
                Root::Host("Option") => Value::Node(Node::new(n.span.start, "option")),
                _ => Value::Unknown,
            }
        } else {
            Value::Unknown
        }
    }

    fn eval_call(&mut self, c: &CallExpression<'_>, depth: usize) -> Value {
        let callee = self.eval_depth(&c.callee, depth + 1);
        let targets = callee.targets();
        let Some(t) = targets.first() else {
            return Value::Unknown;
        };
        let path: Vec<&str> = t.path.iter().map(String::as_str).collect();
        match (&t.root, path.as_slice()) {
            (Root::Document, ["createElement"]) | (Root::Document, ["createElementNS"]) => {
                let index = if path == ["createElementNS"] { 1 } else { 0 };
                let tag = c
                    .arguments
                    .get(index)
                    .and_then(|a| a.as_expression())
                    .map(|e| self.eval_depth(e, depth + 1).strings())
                    .filter(|s| s.len() == 1)
                    .map(|s| s[0].to_ascii_lowercase())
                    .unwrap_or_else(|| "*".into());
                Value::Node(Node::new(c.span.start, &tag))
            }
            (Root::Document, ["querySelector"]) | (Root::Document, ["getElementById"]) => {
                let selector = c
                    .arguments
                    .first()
                    .and_then(|a| a.as_expression())
                    .map(|e| self.eval_depth(e, depth + 1).strings())
                    .unwrap_or_default();
                if path == ["querySelector"] && selector.len() == 1 {
                    let container = match selector[0].as_str() {
                        "html" | ":root" => Some("documentElement"),
                        "body" => Some("body"),
                        "head" => Some("head"),
                        _ => None,
                    };
                    if let Some(container) = container {
                        return Value::Targets(vec![Target::new(Root::Document).child(container)]);
                    }
                }
                // Keep page-selected elements identifiable without treating
                // their ordinary property writes as global mutations.
                Value::Node(Node::new(c.span.start, "selected"))
            }
            // `document.getElementsByTagName('head')[0]` / `body`
            (Root::Document, ["getElementsByTagName"]) => {
                let tag = c
                    .arguments
                    .first()
                    .and_then(|a| a.as_expression())
                    .map(|e| self.eval_depth(e, depth + 1).strings())
                    .unwrap_or_default();
                if tag.len() == 1
                    && builtins::DOCUMENT_CONTAINERS.contains(&tag[0].to_ascii_lowercase().as_str())
                {
                    // Represent `[0]` as the container itself.
                    Value::Array(vec![Value::Targets(vec![
                        Target::new(Root::Document).child(&tag[0].to_ascii_lowercase()),
                    ])])
                } else {
                    Value::Unknown
                }
            }
            (Root::Builtin("Object"), ["assign"]) => {
                // `Object.assign(window, {...})` returns the target.
                c.arguments
                    .first()
                    .and_then(|a| a.as_expression())
                    .map(|e| self.eval_depth(e, depth + 1))
                    .unwrap_or(Value::Unknown)
            }
            (Root::Builtin("String"), []) => c
                .arguments
                .first()
                .and_then(|a| a.as_expression())
                .map(|e| self.eval_depth(e, depth + 1))
                .map(|v| match v.strings() {
                    s if s.is_empty() => Value::Unknown,
                    s => Value::Strings(s),
                })
                .unwrap_or(Value::Unknown),
            _ => Value::Unknown,
        }
    }
}

/// String concatenation of two abstract values. Numbers coerce like JS.
fn concat(a: Value, b: Value, max: usize) -> Value {
    if let (Value::Number(x), Value::Number(y)) = (&a, &b) {
        return Value::Number(x + y);
    }
    let (a, b) = (a.strings(), b.strings());
    if a.is_empty() || b.is_empty() || a.len() * b.len() > max {
        return Value::Unknown;
    }
    let mut out = Vec::with_capacity(a.len() * b.len());
    for x in &a {
        for y in &b {
            out.push(format!("{x}{y}"));
        }
    }
    Value::Strings(out)
}
