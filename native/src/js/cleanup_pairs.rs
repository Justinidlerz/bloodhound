//! Syntactic acquire/release pairs. This narrows persistent-effect candidates;
//! it does not establish that a teardown path executes at runtime.

mod classes;
mod identity;

use super::JsOptions;
use crate::lines::LineIndex;
use crate::model::{Analysis, Rule, Span};
use crate::scheduler::Cancellation;
use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::*};
use oxc_parser::Parser;
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::SourceType;
use oxc_syntax::{node::NodeId, symbol::SymbolId};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Name(String),
    Binding(SymbolId),
    Instance(u32),
    Parameter(Box<Key>, usize),
    Choice(Box<Key>, Box<Key>),
    Index(Box<Key>),
    Member(Box<Key>, Box<Key>),
}

impl Key {
    fn member(self, key: Key) -> Self {
        if self == Key::Name("window".into()) {
            if let Key::Name(name) = &key {
                if let Some(root) = super::builtins::classify(name) {
                    return Key::Name(root.display_name().into());
                }
            }
        }
        Self::Member(Box::new(self), Box::new(key))
    }
    fn property(self, name: &str) -> Self {
        self.member(Self::Name(name.into()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Pair {
    kind: &'static str,
    receiver: Key,
    item: Option<Key>,
    callback: Option<Key>,
}

#[derive(Clone)]
struct Effect {
    span: Span,
    slot: usize,
    scope: Option<NodeId>,
    capture: Option<(u32, Option<NodeId>)>,
    pair: Pair,
    remove: bool,
    parent: Option<Key>,
}

pub(super) fn prune(
    analysis: &mut Analysis,
    source: &str,
    source_type: SourceType,
    options: &JsOptions,
    cancel: &Cancellation,
    map: Option<&sourcemap::DecodedMap>,
) {
    if analysis.findings.is_empty() {
        return;
    }
    let lines = LineIndex::new(source);
    let known: Vec<_> = analysis
        .findings
        .iter()
        .filter_map(|f| known_target(&f.target).map(|key| (lines.position(f.span.start), key)))
        .collect();
    let mut original_targets: HashMap<_, Vec<_>> = HashMap::new();
    if let Some(map) = map {
        for (point, key) in &known {
            if let Some(token) = map.lookup_token(point.0 - 1, point.1) {
                if let (Some(name), Some(view)) = (token.get_source(), token.get_source_view()) {
                    original_targets
                        .entry((name, view.source().as_ptr()))
                        .or_default()
                        .push(((token.get_src_line() + 1, token.get_src_col()), key.clone()));
                }
            }
        }
    }
    let excluded = collect(source, source_type, options, cancel, map, &known);
    analysis
        .findings
        .retain(|f| !eligible(f.rule) || !excluded.contains(&f.span));
    let Some(map) = map else {
        return;
    };
    let lines = LineIndex::new(source);
    let mut originals = HashMap::new();
    analysis.findings.retain(|f| {
        if !eligible(f.rule) || cancel.check().is_err() {
            return true;
        }
        let (line, column) = lines.position(f.span.start);
        let Some(token) = map.lookup_token(line - 1, column) else {
            return true;
        };
        let (Some(name), Some(view)) = (token.get_source(), token.get_source_view()) else {
            return true;
        };
        let ranges = originals
            .entry((name, view.source().as_ptr()))
            .or_insert_with(|| {
                let ty = SourceType::from_path(name)
                    .unwrap_or_else(|_| SourceType::mjs().with_jsx(true))
                    .with_module(true);
                let index = LineIndex::new(view.source());
                collect(
                    view.source(),
                    ty,
                    options,
                    cancel,
                    None,
                    original_targets
                        .get(&(name, view.source().as_ptr()))
                        .map_or(&[], Vec::as_slice),
                )
                .into_iter()
                .map(|s| (index.position(s.start), index.position(s.end)))
                .collect::<Vec<_>>()
            });
        let point = (token.get_src_line() + 1, token.get_src_col());
        !ranges
            .iter()
            .any(|(start, end)| *start <= point && point < *end)
    });
}

fn eligible(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::DocumentWrite
            | Rule::DocumentStyleWrite
            | Rule::DomInsertion
            | Rule::StyleInjection
            | Rule::GlobalListener
            | Rule::GlobalWrite
            | Rule::HostApiOverride
            | Rule::BuiltinMutation
    )
}

fn collect(
    source: &str,
    ty: SourceType,
    options: &JsOptions,
    cancel: &Cancellation,
    map: Option<&sourcemap::DecodedMap>,
    known: &[((u32, u32), Key)],
) -> HashSet<Span> {
    if cancel.check().is_err() {
        return HashSet::new();
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, ty).parse();
    if parsed.fatal_error || parsed.diagnostics.has_errors() {
        return HashSet::new();
    }
    let built = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(false)
        .build(&parsed.program);
    let semantic = &built.semantic;
    if semantic.nodes().len() > options.max_steps {
        return HashSet::new();
    }
    let mut matcher = Matcher {
        semantic,
        definitions: HashMap::new(),
        aliases: HashMap::new(),
        parameters: HashMap::new(),
        indices: HashMap::new(),
        stored: HashMap::new(),
        snapshots: HashMap::new(),
        methods: HashMap::new(),
        owners: HashMap::new(),
        returned_instances: HashMap::new(),
        targets: HashMap::new(),
        literal_iterations: HashMap::new(),
        unstable: HashSet::new(),
        resets: HashMap::new(),
        budget: Cell::new(options.max_steps),
        cancel,
    };
    matcher.prepare();
    let lines = LineIndex::new(source);
    for (point, target) in known {
        let assignment = semantic
            .nodes()
            .iter_enumerated()
            .filter_map(|(_, n)| {
                if let AstKind::AssignmentExpression(a) = n.kind() {
                    (matches!(
                        &a.left,
                        AssignmentTarget::StaticMemberExpression(_)
                            | AssignmentTarget::ComputedMemberExpression(_)
                    ) && lines.position(a.span.start) <= *point
                        && *point < lines.position(a.span.end))
                    .then_some(a)
                } else {
                    None
                }
            })
            .min_by_key(|a| a.span.end - a.span.start);
        if let Some(a) = assignment {
            matcher.targets.insert(a.span.start, target.clone());
        }
    }
    let mut effects = Vec::new();
    for (_, node) in semantic.nodes().iter_enumerated() {
        if !matcher.step() {
            return HashSet::new();
        }
        match node.kind() {
            AstKind::CallExpression(c) => {
                let before = effects.len();
                matcher.call(c, &mut effects);
                for effect in &mut effects[before..] {
                    effect.scope = matcher.scope(c.node_id.get());
                }
            }
            AstKind::AssignmentExpression(a) => matcher.assignment(a, &mut effects),
            _ => {}
        }
    }
    let templates = effects.clone();
    for (_, node) in semantic.nodes().iter_enumerated() {
        if let AstKind::CallExpression(c) = node.kind() {
            matcher.forward(c, &templates, &mut effects);
        }
    }
    let lines = LineIndex::new(source);
    let origin = |span: Span| {
        map.and_then(|m| {
            let (l, c) = lines.position(span.start);
            m.lookup_token(l - 1, c)
        })
        .and_then(|t| t.get_source().map(str::to_owned))
    };
    let mut groups: HashMap<_, Vec<&Effect>> = HashMap::new();
    for effect in &effects {
        groups
            .entry((origin(effect.span), effect.pair.clone()))
            .or_default()
            .push(effect);
    }
    let mut matched = HashMap::new();
    let restored_properties: Vec<_> = groups
        .iter()
        .filter_map(|((origin, pair), group)| {
            (pair.kind == "property"
                && group.iter().any(|e| e.remove)
                && group.iter().any(|e| {
                    !e.remove
                        && group
                            .iter()
                            .any(|r| r.remove && capture_precedes_write(e, r))
                }))
            .then_some((origin.clone(), pair.receiver.clone()))
        })
        .collect();
    for group in groups.values() {
        if !matcher.step() {
            return HashSet::new();
        }
        for effect in group {
            let pair = group.iter().any(|other| {
                effect.remove != other.remove
                    && capture_precedes_write(effect, other)
                    && (effect.pair.kind != "node" || removal_follows_insertion(effect, other))
                    && (effect.pair.kind != "node"
                        || effect.parent.is_none()
                        || other.parent.is_none()
                        || effect.parent == other.parent)
            }) || (effect.pair.kind == "property"
                && restored_properties.iter().any(|(source, parent)| {
                    *source == origin(effect.span) && descendant(&effect.pair.receiver, parent)
                }));
            matched
                .entry((effect.span, effect.slot))
                .and_modify(|any| *any |= pair)
                .or_insert(pair);
        }
    }
    if cancel.check().is_err() {
        return HashSet::new();
    }
    let mut spans = HashMap::new();
    for ((span, _), matched) in matched {
        spans
            .entry(span)
            .and_modify(|all| *all &= matched)
            .or_insert(matched);
    }
    spans
        .into_iter()
        .filter_map(|(span, yes)| yes.then_some(span))
        .collect()
}

struct Matcher<'s, 'a> {
    semantic: &'s Semantic<'a>,
    definitions: HashMap<SymbolId, &'a Expression<'a>>,
    aliases: HashMap<SymbolId, SymbolId>,
    parameters: HashMap<SymbolId, Key>,
    indices: HashMap<Key, Key>,
    stored: HashMap<Key, Key>,
    snapshots: HashMap<Key, (Key, u32, Option<NodeId>)>,
    methods: HashMap<Key, (Key, oxc_span::Span)>,
    owners: HashMap<NodeId, Key>,
    returned_instances: HashMap<SymbolId, Key>,
    targets: HashMap<u32, Key>,
    literal_iterations: HashMap<SymbolId, Vec<Key>>,
    unstable: HashSet<SymbolId>,
    resets: HashMap<SymbolId, Vec<(Option<NodeId>, u32)>>,
    budget: Cell<usize>,
    cancel: &'s Cancellation,
}

impl<'s, 'a> Matcher<'s, 'a> {
    fn effect(
        &self,
        effects: &mut Vec<Effect>,
        span: oxc_span::Span,
        kind: &'static str,
        receiver: Key,
        item: Option<Key>,
        callback: Option<Key>,
        remove: bool,
        parent: Option<Key>,
    ) {
        effects.push(Effect {
            span: span.into(),
            scope: None,
            capture: None,
            slot: effects
                .iter()
                .rev()
                .take_while(|e| e.span == span.into())
                .count(),
            pair: Pair {
                kind,
                receiver,
                item,
                callback,
            },
            remove,
            parent,
        });
    }
    fn call(&self, c: &CallExpression<'a>, effects: &mut Vec<Effect>) {
        let Some((object, method)) = member(&c.callee) else {
            return;
        };
        let Some(receiver) = self.key(object, 0) else {
            return;
        };
        match method {
            "setAttribute" | "removeAttribute" | "setProperty" | "removeProperty" => {
                let Some(key) = arg(c, 0).and_then(|e| self.key(e, 0)) else {
                    return;
                };
                let kind = if matches!(method, "setAttribute" | "removeAttribute") {
                    "attribute"
                } else {
                    "style"
                };
                self.effect(
                    effects,
                    c.span,
                    kind,
                    receiver,
                    Some(key),
                    None,
                    method.starts_with("remove")
                        || (method == "setProperty" && arg(c, 1).is_some_and(reset_style)),
                    None,
                );
            }
            "add" | "remove" => {
                if let Key::Member(base, api) = &receiver {
                    if **api == Key::Name("classList".into()) {
                        let Some(keys) = c
                            .arguments
                            .iter()
                            .map(|a| a.as_expression().and_then(|e| self.key(e, 0)))
                            .collect::<Option<Vec<_>>>()
                        else {
                            return;
                        };
                        for key in keys {
                            self.effect(
                                effects,
                                c.span,
                                "class",
                                *base.clone(),
                                Some(key),
                                None,
                                method == "remove",
                                None,
                            );
                        }
                        return;
                    }
                }
                if method == "remove" && c.arguments.is_empty() {
                    self.effect(effects, c.span, "node", receiver, None, None, true, None);
                }
            }
            "appendChild" | "insertBefore" | "append" | "prepend" | "removeChild" => {
                let count = if matches!(method, "append" | "prepend") {
                    c.arguments.len()
                } else {
                    1
                };
                let Some(keys) = c
                    .arguments
                    .iter()
                    .take(count)
                    .map(|a| a.as_expression().and_then(|e| self.key(e, 0)))
                    .collect::<Option<Vec<_>>>()
                else {
                    return;
                };
                for node in keys {
                    let parent = if receiver == node.clone().property("parentNode")
                        || receiver == node.clone().property("parentElement")
                    {
                        None
                    } else {
                        Some(receiver.clone())
                    };
                    self.effect(
                        effects,
                        c.span,
                        "node",
                        self.stored.get(&node).cloned().unwrap_or(node),
                        None,
                        None,
                        method == "removeChild",
                        parent,
                    );
                }
            }
            "addEventListener" | "removeEventListener" => {
                let (Some(event), Some(callback)) = (
                    arg(c, 0).and_then(|e| self.key(e, 0)),
                    arg(c, 1).and_then(|e| self.key(e, 0)),
                ) else {
                    return;
                };
                let callback = self.stored.get(&callback).cloned().unwrap_or(callback);
                let variants = match &event {
                    Key::Binding(symbol) => self.literal_iterations.get(symbol).cloned(),
                    _ => None,
                }
                .unwrap_or_else(|| vec![event.clone()]);
                for variant in variants {
                    self.effect(
                        effects,
                        c.span,
                        "listener",
                        receiver.clone(),
                        Some(variant.clone()),
                        Some(replace_key(&callback, &event, &variant)),
                        method == "removeEventListener",
                        None,
                    );
                }
            }
            _ => {}
        }
    }
    fn assignment(&self, a: &AssignmentExpression<'a>, effects: &mut Vec<Effect>) {
        if a.operator.as_str() != "=" {
            return;
        }
        let (receiver, property) = match &a.left {
            AssignmentTarget::StaticMemberExpression(m) => (
                self.key(&m.object, 0),
                Some(Key::Name(m.property.name.to_string())),
            ),
            AssignmentTarget::ComputedMemberExpression(m) => {
                (self.key(&m.object, 0), self.key(&m.expression, 0))
            }
            _ => return,
        };
        let (Some(receiver), Some(property)) = (receiver, property) else {
            return;
        };
        if matches!(&receiver,Key::Member(_,p) if **p==Key::Name("style".into())) {
            let reset = reset_style(&a.right);
            self.effect(
                effects,
                a.span,
                "style",
                receiver,
                Some(property),
                None,
                reset,
                None,
            );
        } else {
            let target = self
                .targets
                .get(&a.span.start)
                .cloned()
                .unwrap_or_else(|| receiver.member(property));
            let value = match a.right.get_inner_expression() {
                Expression::Identifier(id) => self.symbol(id).map(Key::Binding),
                _ => self.key(&a.right, 0),
            };
            let snapshot = value
                .and_then(|key| self.snapshots.get(&key))
                .filter(|(saved, _, _)| *saved == target);
            let restored = snapshot.is_some();
            self.effect(
                effects, a.span, "property", target, None, None, restored, None,
            );
            if let Some(effect) = effects.last_mut() {
                effect.scope = self.scope(a.node_id.get());
                effect.capture = snapshot.map(|(_, start, scope)| (*start, *scope));
            }
        }
    }

    fn scope(&self, node: NodeId) -> Option<NodeId> {
        let nodes = self.semantic.nodes();
        for (id, n) in nodes.ancestors_enumerated(node) {
            if !matches!(
                n.kind(),
                AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
            ) {
                continue;
            }
            let mut parent = nodes.parent_id(id);
            while matches!(nodes.kind(parent), AstKind::ParenthesizedExpression(_)) {
                parent = nodes.parent_id(parent);
            }
            if let AstKind::CallExpression(c) = nodes.kind(parent) {
                let immediate = match c.callee.get_inner_expression() {
                    Expression::FunctionExpression(f) => f.node_id.get() == id,
                    Expression::ArrowFunctionExpression(f) => f.node_id.get() == id,
                    _ => false,
                };
                if immediate {
                    continue;
                }
            }
            return Some(id);
        }
        None
    }
    fn left_key(&self, left: &AssignmentTarget<'a>) -> Option<Key> {
        match left {
            AssignmentTarget::StaticMemberExpression(m) => {
                Some(self.key(&m.object, 0)?.property(m.property.name.as_str()))
            }
            AssignmentTarget::ComputedMemberExpression(m) => {
                Some(self.key(&m.object, 0)?.member(self.key(&m.expression, 0)?))
            }
            AssignmentTarget::AssignmentTargetIdentifier(id) => self.symbol(id).map(Key::Binding),
            _ => None,
        }
    }
    fn forward(&self, c: &CallExpression<'a>, templates: &[Effect], effects: &mut Vec<Effect>) {
        let Some(callee) = self.key(&c.callee, 0) else {
            return;
        };
        let Some((instance, body)) = self.methods.get(&callee) else {
            return;
        };
        let args: Vec<_> = c
            .arguments
            .iter()
            .map(|a| a.as_expression().and_then(|e| self.key(e, 0)))
            .collect();
        for template in templates
            .iter()
            .filter(|e| body.start <= e.span.start && e.span.end <= body.end)
        {
            if !self.step() {
                return;
            }
            let substitute = |key: &Key| substitute(key, instance, &args);
            let (Some(receiver), Some(item), Some(callback), Some(parent)) = (
                substitute(&template.pair.receiver),
                substitute_option(&template.pair.item, &substitute),
                substitute_option(&template.pair.callback, &substitute),
                substitute_option(&template.parent, &substitute),
            ) else {
                continue;
            };
            effects.push(Effect {
                span: template.span,
                slot: template.slot,
                scope: template.scope,
                capture: template.capture,
                pair: Pair {
                    kind: template.pair.kind,
                    receiver,
                    item,
                    callback,
                },
                remove: template.remove,
                parent,
            });
        }
    }
}

fn substitute(key: &Key, instance: &Key, args: &[Option<Key>]) -> Option<Key> {
    match key {
        Key::Parameter(owner, index) if **owner == *instance => args.get(*index)?.clone(),
        Key::Member(base, property) => {
            Some(substitute(base, instance, args)?.member(substitute(property, instance, args)?))
        }
        Key::Index(base) => Some(Key::Index(Box::new(substitute(base, instance, args)?))),
        Key::Choice(a, b) => Some(choice(
            substitute(a, instance, args)?,
            substitute(b, instance, args)?,
        )),
        _ => Some(key.clone()),
    }
}
fn substitute_option(
    key: &Option<Key>,
    substitute: &impl Fn(&Key) -> Option<Key>,
) -> Option<Option<Key>> {
    match key {
        Some(key) => substitute(key).map(Some),
        None => Some(None),
    }
}
fn reset_style(e: &Expression<'_>) -> bool {
    let value = match e.get_inner_expression() {
        Expression::StringLiteral(s) => Some(s.value.as_str()),
        Expression::TemplateLiteral(t) if t.expressions.is_empty() => t
            .quasis
            .first()
            .and_then(|q| q.value.cooked.as_ref())
            .map(|s| s.as_str()),
        _ => None,
    };
    value.is_some_and(|s| {
        matches!(
            s,
            "" | "unset" | "initial" | "revert" | "revert-layer" | "auto"
        )
    })
}

fn member<'a>(e: &'a Expression<'a>) -> Option<(&'a Expression<'a>, &'a str)> {
    match e.get_inner_expression() {
        Expression::StaticMemberExpression(m) => Some((&m.object, m.property.name.as_str())),
        _ => None,
    }
}
fn arg<'a>(c: &'a CallExpression<'a>, index: usize) -> Option<&'a Expression<'a>> {
    c.arguments.get(index)?.as_expression()
}

fn choice(a: Key, b: Key) -> Key {
    if a == b {
        a
    } else {
        Key::Choice(Box::new(a), Box::new(b))
    }
}

fn instance_root(key: &Key) -> bool {
    match key {
        Key::Instance(_) => true,
        Key::Member(base, _) => instance_root(base),
        _ => false,
    }
}

fn descendant(key: &Key, parent: &Key) -> bool {
    match key {
        Key::Member(base, _) => **base == *parent || descendant(base, parent),
        _ => false,
    }
}

fn known_target(target: &str) -> Option<Key> {
    if target.contains(['*', '[', ':']) {
        return None;
    }
    let mut parts = target.split('.');
    let root = parts.next()?;
    if super::builtins::classify(root).is_none() {
        return None;
    }
    Some(parts.fold(Key::Name(root.into()), |key, part| key.property(part)))
}

fn capture_precedes_write(a: &Effect, b: &Effect) -> bool {
    let (write, restore) = if a.remove { (b, a) } else { (a, b) };
    restore.capture.is_none_or(|(start, scope)| {
        (scope != write.scope || start < write.span.start)
            && (write.scope != restore.scope || write.span.start < restore.span.start)
    })
}

fn removal_follows_insertion(a: &Effect, b: &Effect) -> bool {
    let (insert, remove) = if a.remove { (b, a) } else { (a, b) };
    insert.scope != remove.scope || insert.span.start < remove.span.start
}

fn replace_key(key: &Key, from: &Key, to: &Key) -> Key {
    if key == from {
        return to.clone();
    }
    match key {
        Key::Member(base, property) => {
            replace_key(base, from, to).member(replace_key(property, from, to))
        }
        _ => key.clone(),
    }
}
