//! Pollution rules.
//!
//! Each public method here is invoked by the visitor for one syntactic form
//! and decides whether it pollutes the shared environment. The decision
//! always starts from an abstract `Target` (see `values.rs`); the rule then
//! classifies by the target's root and path.
//!
//! Classification of a write to `<root>.<path>`:
//!
//! | root / path                       | rule                   |
//! |-----------------------------------|------------------------|
//! | window.<x> (x not host api)       | global-write           |
//! | window.<host api>, <host api>.*   | host-api-override      |
//! | <builtin>.*, *.prototype.*        | builtin-mutation       |
//! | document.<x>, document.body.<x>   | document-write         |
//! | document.(body|head|html).style.* | document-style-write   |
//!
//! Writes whose receiver is unknown, a local object, or an unbound
//! identifier are ignored: they cannot be shown to reach the shared
//! environment.

use super::builtins::{self, DOCUMENT_CONTAINERS, DOCUMENT_WRITE_METHODS, INSERTION_METHODS};
use super::values::{Node, Root, Target, Value};
use super::{Analyzer, ArgKey, Callback, EventKey, ListenerKey, ListenerRemoval};
use crate::model::{DependencyKind, Rule, ScriptMode, Span};
use oxc_ast::ast::*;
use oxc_span::GetSpan;
use oxc_syntax::symbol::SymbolId;

/// Classifies a mutation of `target`. Returns `None` for benign targets.
fn classify_write(target: &Target) -> Option<Rule> {
    let first = target.path.first().map(String::as_str);
    match &target.root {
        Root::Global => match first {
            None => Some(Rule::GlobalWrite), // `window = x` is nonsense but flag it
            Some(name) if builtins::BENIGN_WINDOW_WRITES.contains(&name) => None,
            Some(_) if target.touches_prototype() => Some(Rule::BuiltinMutation),
            Some(_) => Some(Rule::GlobalWrite),
        },
        Root::Document => match first {
            None => Some(Rule::GlobalWrite),
            Some("adoptedStyleSheets") => Some(Rule::StyleInjection),
            Some(name)
                if builtins::BENIGN_DOCUMENT_WRITES.contains(&name) && target.path.len() == 1 =>
            {
                None
            }
            Some(container) if DOCUMENT_CONTAINERS.contains(&container) => {
                match target.path.get(1).map(String::as_str) {
                    Some("style") => Some(Rule::DocumentStyleWrite),
                    Some("innerHTML" | "outerHTML" | "textContent" | "innerText") => {
                        Some(Rule::DomInsertion)
                    }
                    _ => Some(Rule::DocumentWrite),
                }
            }
            Some(_) => Some(Rule::DocumentWrite),
        },
        Root::Builtin(_) => Some(Rule::BuiltinMutation),
        Root::Host(_) => {
            if matches!(target.root, Root::Host("history")) && first == Some("state") {
                return Some(Rule::GlobalWrite);
            }
            // `location.href = …` is navigation, not pollution.
            if matches!(target.root, Root::Host("location")) {
                return None;
            }
            // `console.log = …`-style patching of host objects.
            Some(Rule::HostApiOverride)
        }
    }
}

impl<'s, 'c> Analyzer<'s, 'c> {
    /// Evaluates an assignment target to the abstract targets it denotes.
    fn assignment_targets(&mut self, left: &AssignmentTarget<'_>) -> Vec<Target> {
        match left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => match self.symbol_of(id) {
                Some(symbol) => self.global_binding(symbol).into_iter().collect(),
                None => builtins::classify(&id.name)
                    .map(|root| vec![Target::new(root)])
                    .unwrap_or_else(|| {
                        if self.strict {
                            vec![]
                        } else {
                            vec![Target::new(Root::Global).child(&id.name)]
                        }
                    }),
            },
            AssignmentTarget::StaticMemberExpression(m) => {
                let object = self.eval(&m.object);
                self.write_member(object, m.property.name.as_str())
            }
            AssignmentTarget::ComputedMemberExpression(m) => {
                let object = self.eval(&m.object);
                let keys = self.eval(&m.expression).strings();
                let key = if keys.len() == 1 {
                    keys[0].clone()
                } else {
                    "*".into()
                };
                self.write_member(object, &key)
            }
            _ => vec![],
        }
    }

    /// Writing an object's slot replaces that slot; it does not mutate the
    /// value previously stored there (`{ root: window }.root = {}`).
    fn write_member(&self, receiver: Value, key: &str) -> Vec<Target> {
        if matches!(receiver, Value::Targets(_)) {
            self.member(receiver, key).targets().to_vec()
        } else {
            vec![]
        }
    }

    /// Top-level `var`/`function` in a classic script creates a property of
    /// the global object; this returns that target. Lexical declarations
    /// (`let`/`const`/`class`) live in the shared global lexical scope but
    /// are not reachable through `window`, so they are not reported.
    pub(super) fn global_binding(&self, symbol: SymbolId) -> Option<Target> {
        if self.options.script_mode != ScriptMode::Classic {
            return None;
        }
        if self.scoping.symbol_scope_id(symbol) != self.scoping.root_scope_id() {
            return None;
        }
        let flags = self.scoping.symbol_flags(symbol);
        if flags.is_import() || flags.is_block_scoped() || flags.is_class() {
            return None;
        }
        Some(Target::new(Root::Global).child(self.scoping.symbol_name(symbol)))
    }

    /// `a = b`, `a.b = c`, `a[b] = c`, compound forms.
    pub(super) fn rule_assignment(&mut self, a: &AssignmentExpression<'_>) {
        self.uncertain = false;
        let value = self.eval(&a.right);
        let targets = self.assignment_targets(&a.left);
        let span: Span = a.span().into();
        for target in &targets {
            // Node property writes: track `src`/`href` and inline code for
            // dependency discovery, and flag `document.body.innerHTML`.
            if let Some(rule) = classify_write(target) {
                let detail = value.type_name().map(|t| t.to_string());
                let op = if a.operator.as_str() == "=" {
                    "write"
                } else {
                    "update"
                };
                self.finding(rule, span, target.dotted(), op, detail);
            }
        }
        // Propagate the value into a local binding so aliases resolve later.
        if let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left {
            if let Some(symbol) = self.symbol_of(id) {
                if a.operator.as_str() == "=" {
                    self.values.insert(symbol, value.clone());
                } else {
                    self.values.remove(&symbol);
                }
            }
        }
        // Property write on a tracked DOM node: `script.src = url`.
        if let AssignmentTarget::StaticMemberExpression(m) = &a.left {
            if let Expression::Identifier(id) = m.object.get_inner_expression() {
                if let Some(symbol) = self.symbol_of(id) {
                    self.node_property_write(symbol, m.property.name.as_str(), &value, span);
                    if let Some(Value::Object(map)) = self.values.get_mut(&symbol) {
                        map.insert(m.property.name.to_string(), value.clone());
                    }
                }
            }
        }
    }

    /// Records a dependency when a created `<script>`/`<link>` node gets a
    /// URL, and remembers inline code assigned to a script element.
    fn node_property_write(&mut self, symbol: SymbolId, prop: &str, value: &Value, span: Span) {
        let Some(Value::Node(node)) = self.values.get(&symbol).cloned() else {
            return;
        };
        match (node.tag.as_str(), prop) {
            ("script", "text" | "textContent" | "innerHTML") => {
                self.gap("dynamic-inline-script", span, None);
            }
            _ => {}
        }
        let mut node = node;
        match prop {
            "src" | "href" => node.urls = value.strings(),
            "rel" => node.rel = value.strings().join(" ").to_ascii_lowercase(),
            "type" => node.module = value.strings() == ["module"],
            _ => return,
        }
        for alias in self.values.values_mut() {
            if matches!(alias, Value::Node(n) if n.id == node.id) {
                *alias = Value::Node(node.clone());
            }
        }
    }

    /// `delete window.foo`, `delete Array.prototype.x`.
    pub(super) fn rule_delete(&mut self, u: &UnaryExpression<'_>) {
        self.uncertain = false;
        let targets = match u.argument.get_inner_expression() {
            Expression::StaticMemberExpression(m) => {
                let object = self.eval(&m.object);
                self.write_member(object, &m.property.name)
            }
            Expression::ComputedMemberExpression(m) => {
                let object = self.eval(&m.object);
                let keys = self.eval(&m.expression).strings();
                self.write_member(object, if keys.len() == 1 { &keys[0] } else { "*" })
            }
            _ => vec![],
        };
        for target in &targets {
            if let Some(rule) = classify_write(target) {
                self.finding(rule, u.span().into(), target.dotted(), "delete", None);
            }
        }
    }

    /// `window.counter++` and friends.
    pub(super) fn rule_update(&mut self, u: &UpdateExpression<'_>) {
        self.uncertain = false;
        let targets = match &u.argument {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => match self.symbol_of(id) {
                Some(symbol) => self.global_binding(symbol).into_iter().collect(),
                None => builtins::classify(&id.name)
                    .map(|root| vec![Target::new(root)])
                    .unwrap_or_default(),
            },
            other => other
                .as_member_expression()
                .map(|m| match m {
                    MemberExpression::StaticMemberExpression(m) => {
                        let object = self.eval(&m.object);
                        self.write_member(object, m.property.name.as_str())
                    }
                    MemberExpression::ComputedMemberExpression(m) => {
                        let object = self.eval(&m.object);
                        let keys = self.eval(&m.expression).strings();
                        let key = if keys.len() == 1 {
                            keys[0].clone()
                        } else {
                            "*".into()
                        };
                        self.write_member(object, &key)
                    }
                    _ => vec![],
                })
                .unwrap_or_default(),
        };
        for target in &targets {
            if let Some(rule) = classify_write(target) {
                self.finding(
                    rule,
                    u.span().into(),
                    target.dotted(),
                    "update",
                    Some("number".into()),
                );
            }
        }
    }

    /// Top-level declarations in classic scripts.
    pub(super) fn rule_declaration(
        &mut self,
        ident: &BindingIdentifier<'_>,
        value_type: Option<&'static str>,
    ) {
        let Some(symbol) = ident.symbol_id.get() else {
            return;
        };
        if let Some(target) = self.global_binding(symbol) {
            self.uncertain = false;
            self.finding(
                Rule::GlobalWrite,
                ident.span().into(),
                target.dotted(),
                "declare",
                value_type.map(String::from),
            );
        }
    }

    /// Call expressions: DOM insertion, style injection, listeners,
    /// `Object.defineProperty`, `Object.assign`, `importScripts`, `eval`.
    pub(super) fn rule_call(&mut self, c: &CallExpression<'_>) {
        self.uncertain = false;
        let callee = c.callee.get_inner_expression();

        // Bare calls: importScripts(...), eval(...), Function(...).
        if let Expression::Identifier(id) = callee {
            // Record calls to known local helpers so registrations inside
            // them can pair through the arguments passed here.
            if let Some(symbol) = self.symbol_of(id) {
                let args = c
                    .arguments
                    .iter()
                    .map(|a| a.as_expression().and_then(|e| self.arg_key(e)))
                    .collect();
                self.fn_calls.push((symbol, args));
            }
            if self.symbol_of(id).is_none()
                && matches!(id.name.as_str(), "addEventListener" | "removeEventListener")
            {
                let args: Vec<Value> = c
                    .arguments
                    .iter()
                    .map(|a| {
                        a.as_expression()
                            .map(|e| self.eval(e))
                            .unwrap_or(Value::Unknown)
                    })
                    .collect();
                if id.name == "addEventListener" {
                    self.listener(&Target::new(Root::Global), &args, c, c.span().into());
                } else {
                    self.record_listener_removal(&args, c);
                }
                return;
            }
            if self.symbol_of(id).is_none() {
                match id.name.as_str() {
                    "importScripts" => {
                        for arg in &c.arguments {
                            if let Some(e) = arg.as_expression() {
                                let v = self.eval(e);
                                self.dependency(
                                    DependencyKind::Worker,
                                    c.span().into(),
                                    &v,
                                    Some(ScriptMode::Classic),
                                );
                            }
                        }
                        return;
                    }
                    "eval" | "Function" => {
                        self.gap("dynamic-code-execution", c.span().into(), None);
                        return;
                    }
                    _ => {}
                }
            }
        }

        // Member calls: receiver.method(args)
        let (receiver, method) = match callee {
            Expression::StaticMemberExpression(m) => {
                (self.eval(&m.object), m.property.name.to_string())
            }
            Expression::ComputedMemberExpression(m) => {
                let object = self.eval(&m.object);
                let keys = self.eval(&m.expression).strings();
                if keys.len() != 1 {
                    return;
                }
                (object, keys[0].clone())
            }
            Expression::ChainExpression(ch) => match &ch.expression {
                ChainElement::StaticMemberExpression(m) => {
                    (self.eval(&m.object), m.property.name.to_string())
                }
                _ => return,
            },
            _ => return,
        };
        let args: Vec<Value> = c
            .arguments
            .iter()
            .map(|a| {
                a.as_expression()
                    .map(|e| self.eval(e))
                    .unwrap_or(Value::Unknown)
            })
            .collect();
        let span: Span = c.span().into();

        // Record every removal for end-of-file pairing, including ones on
        // receivers the target abstraction cannot resolve.
        if method == "removeEventListener" {
            self.record_listener_removal(&args, c);
        }

        // Calls on script-created nodes: `style.remove()`, `el.appendChild(child)`.
        if method == "removeChild" {
            if let Some(Value::Node(node)) = args.first() {
                if matches!(receiver, Value::Targets(_) | Value::Node(_)) {
                    self.cleanup(node.id);
                }
            }
        }
        if let Value::Node(node) = &receiver {
            if method == "remove" {
                self.cleanup(node.id);
            }
            if let Some(symbol) = self.callee_object_symbol(callee) {
                if method == "setAttribute" {
                    if let (Some(key), Some(value)) = (
                        args.first().and_then(|v| v.strings().into_iter().next()),
                        args.get(1),
                    ) {
                        self.node_property_write(symbol, &key, value, span);
                    }
                }
            }
            if node.tag == "selected" && INSERTION_METHODS.contains(&method.as_str()) {
                self.dom_insertion(
                    c,
                    &args,
                    &Target::new(Root::Document).child("selected"),
                    &method,
                    span,
                );
            }
            return;
        }

        let receivers = receiver.targets().to_vec();
        for target in &receivers {
            match (&target.root, target.path.as_slice()) {
                // Object.defineProperty(window, 'x', …) / Reflect.set / Object.assign
                (Root::Builtin("Object" | "Reflect"), [])
                    if matches!(
                        method.as_str(),
                        "defineProperty"
                            | "defineProperties"
                            | "assign"
                            | "set"
                            | "deleteProperty"
                            | "setPrototypeOf"
                    ) =>
                {
                    let api = format!("{}.{method}", target.root.display_name());
                    let Some(Value::Targets(base_targets)) = args.first() else {
                        continue;
                    };
                    let operation = match method.as_str() {
                        "defineProperty" | "defineProperties" => "define",
                        "deleteProperty" => "delete",
                        _ => "write",
                    };
                    // Keys come from the descriptor object (defineProperties /
                    // assign) or the second argument (defineProperty / set).
                    let keys: Vec<String> = match method.as_str() {
                        "assign" => args
                            .iter()
                            .skip(1)
                            .flat_map(|v| match v {
                                Value::Object(map) => map.keys().cloned().collect::<Vec<_>>(),
                                _ => vec!["*".into()],
                            })
                            .collect(),
                        "defineProperties" => match args.get(1) {
                            Some(Value::Object(map)) => map.keys().cloned().collect(),
                            _ => vec!["*".into()],
                        },
                        "setPrototypeOf" => vec!["__proto__".into()],
                        _ => {
                            let keys = args.get(1).map(|v| v.strings()).unwrap_or_default();
                            if keys.is_empty() {
                                vec!["*".into()]
                            } else {
                                keys
                            }
                        }
                    };
                    for base in base_targets {
                        for key in &keys {
                            let member = self.member(Value::Targets(vec![base.clone()]), key);
                            for t in member.targets() {
                                if let Some(rule) = classify_write(t) {
                                    self.finding(
                                        rule,
                                        span,
                                        t.dotted(),
                                        operation,
                                        Some(api.clone()),
                                    );
                                }
                            }
                        }
                    }
                }
                // document.write / document.writeln
                (Root::Document, []) if DOCUMENT_WRITE_METHODS.contains(&method.as_str()) => {
                    self.finding(
                        Rule::DomInsertion,
                        span,
                        "document".into(),
                        "write",
                        Some(format!("document.{method}")),
                    );
                }
                // document.adoptedStyleSheets.push(sheet)
                (Root::Document, [p])
                    if p == "adoptedStyleSheets"
                        && matches!(method.as_str(), "push" | "unshift" | "splice") =>
                {
                    self.finding(
                        Rule::StyleInjection,
                        span,
                        "document.adoptedStyleSheets".into(),
                        "insert",
                        None,
                    );
                }
                // document.body.appendChild(node) etc.
                (Root::Document, [container])
                    if DOCUMENT_CONTAINERS.contains(&container.as_str()) =>
                {
                    if INSERTION_METHODS.contains(&method.as_str()) {
                        self.dom_insertion(c, &args, target, &method, span);
                    } else if method == "setAttribute" {
                        let attr = args.first().map(|v| v.strings()).unwrap_or_default();
                        if attr.len() == 1 && attr[0] == "style" {
                            self.finding(
                                Rule::DocumentStyleWrite,
                                span,
                                format!("{}.style", target.dotted()),
                                "style",
                                Some("setAttribute".into()),
                            );
                        } else {
                            self.finding(
                                Rule::DocumentWrite,
                                span,
                                format!(
                                    "{}[{}]",
                                    target.dotted(),
                                    attr.first().map(String::as_str).unwrap_or("*")
                                ),
                                "write",
                                Some("setAttribute".into()),
                            );
                        }
                    } else if matches!(method.as_str(), "addEventListener") {
                        self.listener(target, &args, c, span);
                    }
                }
                // document.body.classList.add / document.body.style.setProperty
                (Root::Document, [container, api])
                    if DOCUMENT_CONTAINERS.contains(&container.as_str()) =>
                {
                    match api.as_str() {
                        "classList"
                            if matches!(
                                method.as_str(),
                                "add" | "remove" | "toggle" | "replace"
                            ) =>
                        {
                            let class = args
                                .first()
                                .map(|v| v.strings().join("|"))
                                .unwrap_or_default();
                            self.finding(
                                Rule::DocumentWrite,
                                span,
                                format!("{}.classList", target.dotted_prefix(1)),
                                "write",
                                Some(format!("classList.{method}({class})")),
                            );
                        }
                        "style" if matches!(method.as_str(), "setProperty" | "removeProperty") => {
                            let prop = args
                                .first()
                                .map(|v| v.strings().join("|"))
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| "*".into());
                            self.finding(
                                Rule::DocumentStyleWrite,
                                span,
                                format!("{}.{prop}", target.dotted()),
                                "style",
                                Some(format!("style.{method}")),
                            );
                        }
                        _ => {}
                    }
                }
                // Element.prototype.append etc. handled by classify_write on assignment; here calls on
                // `document.head.querySelector(...)` are unknown receivers.
                // window.addEventListener / removeEventListener
                (Root::Global, []) if method == "addEventListener" => {
                    self.listener(target, &args, c, span)
                }
                // document.addEventListener
                (Root::Document, []) if method == "addEventListener" => {
                    self.listener(target, &args, c, span)
                }
                // document.styleSheets[0].insertRule(...)
                (Root::Document, [p, ..])
                    if p == "styleSheets"
                        && matches!(method.as_str(), "insertRule" | "addRule") =>
                {
                    self.finding(
                        Rule::StyleInjection,
                        span,
                        target.dotted(),
                        "insert",
                        Some(method.clone()),
                    );
                }
                // customElements.define('x-foo', …)
                (Root::Host("customElements"), []) if method == "define" => {
                    let name = args
                        .first()
                        .map(|v| v.strings().join("|"))
                        .unwrap_or_else(|| "*".into());
                    self.finding(
                        Rule::GlobalWrite,
                        span,
                        format!("customElements.{name}"),
                        "define",
                        Some("customElements.define".into()),
                    );
                }
                _ => {}
            }
        }
    }

    /// `receiver.appendChild(arg)` where receiver is a document container.
    fn dom_insertion(
        &mut self,
        c: &CallExpression<'_>,
        args: &[Value],
        container: &Target,
        method: &str,
        span: Span,
    ) {
        // insertBefore(node, ref) / insertAdjacentHTML(pos, html): the
        // inserted thing is the first argument except for insertAdjacent*.
        let (arg_index, markup) = match method {
            "insertAdjacentHTML" | "insertAdjacentText" => (usize::MAX, true),
            "insertAdjacentElement" => (1, false),
            _ => (0, false),
        };
        if markup {
            self.finding(
                Rule::DomInsertion,
                span,
                container.dotted(),
                "insert",
                Some(method.to_string()),
            );
            return;
        }
        let inserted = args.get(arg_index);
        let node_symbol = match inserted {
            Some(Value::Node(n)) => Some(n.id),
            _ => None,
        };
        match inserted {
            Some(Value::Node(Node {
                tag,
                urls,
                rel,
                module,
                ..
            })) => {
                let detail = if urls.is_empty() {
                    format!("<{tag}>")
                } else {
                    format!("<{tag}> {}", urls.join(" | "))
                };
                if tag == "script" && !urls.is_empty() {
                    self.dependency(
                        DependencyKind::DynamicScript,
                        span,
                        &Value::Strings(urls.clone()),
                        Some(if *module {
                            ScriptMode::Module
                        } else {
                            ScriptMode::Classic
                        }),
                    );
                }
                let stylesheet = tag == "link" && rel.split_whitespace().any(|r| r == "stylesheet");
                if stylesheet && !urls.is_empty() {
                    self.dependency(
                        DependencyKind::DynamicStyle,
                        span,
                        &Value::Strings(urls.clone()),
                        None,
                    );
                }
                match tag.as_str() {
                    "style" => {
                        // Style injection is only pollution if the node is
                        // never removed in this resource.
                        self.deferred_finding(
                            Rule::StyleInjection,
                            span,
                            container.dotted(),
                            "insert",
                            Some(detail),
                            node_symbol,
                            None,
                        );
                    }
                    "link" if stylesheet => self.deferred_finding(
                        Rule::StyleInjection,
                        span,
                        container.dotted(),
                        "insert",
                        Some(detail),
                        node_symbol,
                        None,
                    ),
                    "script" => {
                        // A script tag insertion loads code; that is the
                        // dependency we already recorded via `src`. It is also
                        // a DOM insertion but scripts are expected to be
                        // removed by loaders, so defer.
                        self.deferred_finding(
                            Rule::DomInsertion,
                            span,
                            container.dotted(),
                            "insert",
                            Some(detail),
                            node_symbol,
                            None,
                        );
                    }
                    _ => self.deferred_finding(
                        Rule::DomInsertion,
                        span,
                        container.dotted(),
                        "insert",
                        Some(detail),
                        node_symbol,
                        None,
                    ),
                }
            }
            _ => {
                // Unknown node: still an insertion into a document container.
                self.deferred_finding(
                    Rule::DomInsertion,
                    span,
                    container.dotted(),
                    "insert",
                    Some(method.to_string()),
                    node_symbol,
                    None,
                );
            }
        }
        if matches!(method, "append" | "prepend" | "replaceChildren") && args.len() > 1 {
            self.dom_insertion(c, &args[1..], container, method, span);
        }
    }

    /// Identity of a callback reference for listener pairing: the binding
    /// itself for local identifiers, or the path for global references and
    /// `this.handler` / `obj.handler` (rooted at the binding when it resolves). Anonymous
    /// functions and calls such as `fn.bind(this)` have no stable identity.
    pub(super) fn callback_identity(&self, expr: &Expression<'_>) -> Option<Callback> {
        match expr.get_inner_expression() {
            Expression::Identifier(id) => {
                let Some(symbol) = self.symbol_of(id) else {
                    return Some(Callback::Member(id.name.to_string()));
                };
                if let Some(alias) = self.aliases.get(&symbol) {
                    return Some(alias.clone());
                }
                Some(Callback::Binding(symbol))
            }
            Expression::StaticMemberExpression(_) => self.member_path(expr).map(Callback::Member),
            _ => None,
        }
    }

    fn member_path(&self, expr: &Expression<'_>) -> Option<String> {
        match expr.get_inner_expression() {
            Expression::ThisExpression(_) => Some("this".into()),
            Expression::Identifier(id) => Some(match self.symbol_of(id) {
                Some(symbol) if self.this_aliases.contains(&symbol) => "this".into(),
                Some(symbol) => match self.aliases.get(&symbol) {
                    Some(Callback::Binding(root)) => format!("sym:{root:?}"),
                    Some(Callback::Member(path)) => path.clone(),
                    None => format!("sym:{symbol:?}"),
                },
                None => id.name.to_string(),
            }),
            Expression::StaticMemberExpression(m) => Some(format!(
                "{}.{}",
                self.member_path(&m.object)?,
                m.property.name
            )),
            _ => None,
        }
    }

    /// Identity of one call argument for helper-parameter substitution.
    pub(super) fn arg_key(&mut self, expr: &Expression<'_>) -> Option<ArgKey> {
        let strings = self.eval(expr).strings();
        if strings.len() == 1 {
            Some(ArgKey::Literal(strings[0].clone()))
        } else {
            self.callback_identity(expr).map(ArgKey::Ref)
        }
    }

    fn record_listener_removal(&mut self, args: &[Value], call: &CallExpression<'_>) {
        if let Some(key) = self.listener_key(args, call) {
            self.listener_removals.push(ListenerRemoval {
                key,
                owner: self.fn_stack.last().copied().flatten(),
            });
        }
    }

    /// `window.addEventListener(event, …)` — deferred until the file end so
    /// a matching `removeEventListener` cancels it.
    pub(super) fn listener_key(
        &self,
        args: &[Value],
        call: &CallExpression<'_>,
    ) -> Option<ListenerKey> {
        let event_expr = call.arguments.first()?.as_expression()?;
        let events = args.first()?.strings();
        let event = if events.len() == 1 {
            EventKey::Literal(events[0].clone())
        } else {
            EventKey::Ref(self.callback_identity(event_expr.get_inner_expression())?)
        };
        let callback = self.callback_identity(
            call.arguments
                .get(1)?
                .as_expression()?
                .get_inner_expression(),
        )?;
        Some(ListenerKey { event, callback })
    }

    fn listener(
        &mut self,
        receiver: &Target,
        args: &[Value],
        call: &CallExpression<'_>,
        span: Span,
    ) {
        // Anonymous inline callbacks have no reference that a later
        // `removeEventListener` could name, so they are outside this rule.
        if let Some(callback) = call.arguments.get(1).and_then(|a| a.as_expression()) {
            if matches!(
                callback.get_inner_expression(),
                Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
            ) {
                return;
            }
        }
        // `once` and `signal` are removal mechanisms of their own: the
        // listener unregisters after firing or on abort.
        if let Some(Value::Object(map)) = args.get(2) {
            let once = matches!(map.get("once"), Some(Value::Bool(true)));
            let signal = map
                .get("signal")
                .is_some_and(|v| !matches!(v, Value::Undefined | Value::Null));
            if once || signal {
                return;
            }
        }
        let events = args.first().map(|v| v.strings()).unwrap_or_default();
        let event = events.first().cloned().unwrap_or_else(|| "*".into());
        let key = self.listener_key(args, call);
        self.deferred_finding(
            Rule::GlobalListener,
            span,
            format!("{}:{event}", receiver.dotted()),
            "listen",
            None,
            None,
            key,
        );
    }

    /// Symbol of `x` in `x.method(...)`.
    fn callee_object_symbol(&self, callee: &Expression<'_>) -> Option<SymbolId> {
        let object = match callee {
            Expression::StaticMemberExpression(m) => &m.object,
            Expression::ComputedMemberExpression(m) => &m.object,
            _ => return None,
        };
        match object.get_inner_expression() {
            Expression::Identifier(id) => self.symbol_of(id),
            _ => None,
        }
    }

    /// `new Worker(url, { type })` / `new SharedWorker(url)`.
    pub(super) fn rule_new(&mut self, n: &NewExpression<'_>) {
        let callee = self.eval(&n.callee);
        let Some(t) = callee.targets().first() else {
            return;
        };
        if !t.path.is_empty() {
            return;
        }
        if let Root::Host("Worker" | "SharedWorker") = t.root {
            let url = n
                .arguments
                .first()
                .and_then(|a| a.as_expression())
                .map(|e| self.eval(e))
                .unwrap_or(Value::Unknown);
            let module = n
                .arguments
                .get(1)
                .and_then(|a| a.as_expression())
                .map(|e| self.eval(e))
                .is_some_and(
                    |v| matches!(v.property("type"), Value::Strings(s) if s == ["module"]),
                );
            let mode = if module {
                ScriptMode::Module
            } else {
                ScriptMode::Classic
            };
            self.dependency(DependencyKind::Worker, n.span().into(), &url, Some(mode));
        }
    }
}

impl Target {
    /// Dotted form of the first `n` path segments.
    fn dotted_prefix(&self, n: usize) -> String {
        let mut s = self.root.display_name().to_string();
        for p in self.path.iter().take(n) {
            s.push('.');
            s.push_str(p);
        }
        s
    }
}
