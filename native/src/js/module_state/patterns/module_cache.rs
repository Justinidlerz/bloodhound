//! Track successful import results through local callbacks and relative loader
//! helpers embedded in the source map. A cached promise is deliberately retained.

use super::*;
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;

#[derive(Clone, PartialEq, Eq)]
enum Origin {
    Unknown,
    Missing,
    Module,
    Promise,
    Loader,
    Object(HashMap<String, Origin>),
}

type Environment = HashMap<SymbolId, Origin>;

impl Patterns<'_, '_> {
    pub(super) fn module_cache(&self, symbol: SymbolId) -> bool {
        if self.initial(symbol) != Some(Literal::Undefined) || self.exported(symbol) {
            return false;
        }
        let Some(writes) = self.writes(symbol) else {
            return false;
        };
        if writes.is_empty() {
            return false;
        }
        let environment = HashMap::from([(symbol, Origin::Module)]);
        writes.iter().all(|(_, a)| {
            a.operator.as_str() == "=" && self.origin(&a.right, &environment, 0) == Origin::Module
        })
    }

    fn origin(
        &self,
        expression: &Expression<'_>,
        environment: &Environment,
        depth: usize,
    ) -> Origin {
        if depth > 20 || !self.step() {
            return Origin::Unknown;
        }
        match expression.get_inner_expression() {
            Expression::ImportExpression(_) => Origin::Promise,
            Expression::AwaitExpression(a) => {
                if self.origin(&a.argument, environment, depth + 1) == Origin::Promise {
                    Origin::Module
                } else {
                    Origin::Unknown
                }
            }
            Expression::Identifier(id) => {
                let Some(symbol) = self.symbol(id) else {
                    return if id.name == "undefined" {
                        Origin::Missing
                    } else {
                        Origin::Unknown
                    };
                };
                if let Some(value) = environment.get(&symbol) {
                    return value.clone();
                }
                if let Some(function) = self.functions.get(&symbol) {
                    return if self.invoke(*function, &[], environment, depth + 1) == Origin::Promise
                    {
                        Origin::Loader
                    } else {
                        Origin::Unknown
                    };
                }
                if self
                    .semantic
                    .scoping()
                    .get_resolved_references(symbol)
                    .any(|r| r.is_write())
                {
                    return Origin::Unknown;
                }
                if let Some(value) = self.definitions.get(&symbol) {
                    return self.origin(value, environment, depth + 1);
                }
                self.parameter_origin(symbol, environment, depth + 1)
            }
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => {
                if self.invoke(
                    function_node(expression).unwrap(),
                    &[],
                    environment,
                    depth + 1,
                ) == Origin::Promise
                {
                    Origin::Loader
                } else {
                    Origin::Unknown
                }
            }
            Expression::ConditionalExpression(c) => {
                let a = self.origin(&c.consequent, environment, depth + 1);
                if a == self.origin(&c.alternate, environment, depth + 1) {
                    a
                } else {
                    Origin::Unknown
                }
            }
            Expression::ObjectExpression(o) => {
                let mut properties = HashMap::new();
                for property in &o.properties {
                    let ObjectPropertyKind::ObjectProperty(p) = property else {
                        return Origin::Unknown;
                    };
                    if p.computed {
                        return Origin::Unknown;
                    }
                    let Some(key) = p.key.static_name() else {
                        return Origin::Unknown;
                    };
                    properties.insert(
                        key.to_string(),
                        self.origin(&p.value, environment, depth + 1),
                    );
                }
                Origin::Object(properties)
            }
            Expression::CallExpression(c) => {
                let args: Vec<_> = c
                    .arguments
                    .iter()
                    .map(|a| {
                        a.as_expression()
                            .map_or(Origin::Unknown, |e| self.origin(e, environment, depth + 1))
                    })
                    .collect();
                if let Expression::StaticMemberExpression(m) = c.callee.get_inner_expression() {
                    if m.property.name == "resolve"
                        && matches!(m.object.get_inner_expression(), Expression::Identifier(id) if id.name == "Promise" && self.symbol(id).is_none())
                        && args.as_slice() == [Origin::Module]
                    {
                        return Origin::Promise;
                    }
                    if m.property.name == "then"
                        && self.origin(&m.object, environment, depth + 1) == Origin::Promise
                    {
                        // A rejection handler may turn an error into ordinary data.
                        if c.arguments.len() != 1 {
                            return Origin::Unknown;
                        }
                        let callback = c.arguments[0]
                            .as_expression()
                            .and_then(|e| self.function(e));
                        return if callback.is_some_and(|f| {
                            matches!(
                                self.invoke(f, &[Origin::Module], environment, depth + 1),
                                Origin::Module | Origin::Promise
                            )
                        }) {
                            Origin::Promise
                        } else {
                            Origin::Unknown
                        };
                    }
                }
                if let Expression::Identifier(id) = c.callee.get_inner_expression() {
                    if self.symbol(id).and_then(|s| environment.get(&s)) == Some(&Origin::Loader) {
                        return Origin::Promise;
                    }
                    if let Some(symbol) = self.symbol(id) {
                        if let Some(function) = self.functions.get(&symbol) {
                            return self.invoke(*function, &args, environment, depth + 1);
                        }
                        return self.imported_call(symbol, &args, depth + 1);
                    }
                }
                self.function(&c.callee).map_or(Origin::Unknown, |f| {
                    self.invoke(f, &args, environment, depth + 1)
                })
            }
            _ => Origin::Unknown,
        }
    }

    fn invoke(
        &self,
        function: NodeId,
        args: &[Origin],
        environment: &Environment,
        depth: usize,
    ) -> Origin {
        if depth > 20 || !self.step() {
            return Origin::Unknown;
        }
        let (params, body, is_async) = match self.semantic.nodes().kind(function) {
            AstKind::Function(f) if !f.generator => {
                (&f.params, f.body.as_ref().map(|b| &b.statements), f.r#async)
            }
            AstKind::ArrowFunctionExpression(f) => (
                &f.params,
                match &f.body {
                    ArrowFunctionBody::FunctionBody(b) => Some(&b.statements),
                    _ => None,
                },
                f.r#async,
            ),
            _ => return Origin::Unknown,
        };
        if body.is_some_and(|body| !matches!(body.last(), Some(Statement::ReturnStatement(_)))) {
            return Origin::Unknown;
        }
        let mut environment = environment.clone();
        let inherited: HashSet<_> = environment.keys().copied().collect();
        for (index, param) in params.items.iter().enumerate() {
            let mut value = args.get(index).cloned().unwrap_or(Origin::Missing);
            if value == Origin::Missing {
                if let Some(default) = &param.initializer {
                    value = self.origin(default, &environment, depth + 1);
                }
            }
            self.bind_origin(&param.pattern, value, &mut environment, depth + 1);
        }
        if environment
            .keys()
            .filter(|symbol| !inherited.contains(symbol))
            .any(|symbol| {
                self.semantic
                    .scoping()
                    .get_resolved_references(*symbol)
                    .any(|r| r.is_write())
            })
        {
            return Origin::Unknown;
        }
        let returns = self.returned(function);
        let mut values = returns
            .iter()
            .map(|e| self.origin(e, &environment, depth + 1));
        let Some(first) = values.next() else {
            return Origin::Unknown;
        };
        if values.any(|value| value != first) {
            return Origin::Unknown;
        }
        if is_async && first == Origin::Module {
            Origin::Promise
        } else {
            first
        }
    }

    fn bind_origin(
        &self,
        pattern: &BindingPattern<'_>,
        value: Origin,
        environment: &mut Environment,
        depth: usize,
    ) {
        if depth > 20 || !self.step() {
            return;
        }
        match pattern {
            BindingPattern::BindingIdentifier(id) => {
                if let Some(symbol) = id.symbol_id.get() {
                    environment.insert(symbol, value);
                }
            }
            BindingPattern::AssignmentPattern(a) => {
                let value = if value == Origin::Missing {
                    self.origin(&a.right, environment, depth + 1)
                } else {
                    value
                };
                self.bind_origin(&a.left, value, environment, depth + 1);
            }
            BindingPattern::ObjectPattern(o) => {
                for p in &o.properties {
                    let value = match (&value, p.key.static_name()) {
                        (Origin::Object(properties), Some(key)) if !p.computed => properties
                            .get(key.as_ref())
                            .cloned()
                            .unwrap_or(Origin::Missing),
                        _ => Origin::Unknown,
                    };
                    self.bind_origin(&p.value, value, environment, depth + 1);
                }
            }
            _ => {}
        }
    }

    fn parameter_origin(
        &self,
        symbol: SymbolId,
        environment: &Environment,
        depth: usize,
    ) -> Origin {
        if depth > 20 || !self.step() {
            return Origin::Unknown;
        }
        let Some(owner) = self.owner(self.semantic.scoping().symbol_declaration(symbol)) else {
            return Origin::Unknown;
        };
        let params = match self.semantic.nodes().kind(owner) {
            AstKind::Function(f) => &f.params,
            AstKind::ArrowFunctionExpression(f) => &f.params,
            _ => return Origin::Unknown,
        };
        // Only the fulfillment value (first argument) is a module namespace.
        if !matches!(params.items.first().map(|p| &p.pattern), Some(BindingPattern::BindingIdentifier(id)) if id.symbol_id.get() == Some(symbol))
        {
            return Origin::Unknown;
        }
        let Some((&function_symbol, _)) = self.functions.iter().find(|(_, node)| **node == owner)
        else {
            return Origin::Unknown;
        };
        if self.exported(function_symbol) {
            return Origin::Unknown;
        }
        let mut seen = false;
        for reference in self
            .semantic
            .scoping()
            .get_resolved_references(function_symbol)
        {
            if reference.is_write() {
                return Origin::Unknown;
            }
            let AstKind::CallExpression(call) =
                self.semantic.nodes().parent_kind(reference.node_id())
            else {
                return Origin::Unknown;
            };
            let Expression::StaticMemberExpression(member) = call.callee.get_inner_expression()
            else {
                return Origin::Unknown;
            };
            if member.property.name != "then"
                || !call
                    .arguments
                    .first()
                    .and_then(Argument::as_expression)
                    .is_some_and(|e| self.expression_symbol(e) == Some(function_symbol))
                || self.origin(&member.object, environment, depth + 1) != Origin::Promise
            {
                return Origin::Unknown;
            }
            seen = true;
        }
        if seen {
            Origin::Module
        } else {
            Origin::Unknown
        }
    }

    fn imported_call(&self, symbol: SymbolId, args: &[Origin], depth: usize) -> Origin {
        if depth > 12 || !self.step() {
            return Origin::Unknown;
        }
        let Some((map, filename)) = self.sources else {
            return Origin::Unknown;
        };
        let node = self.semantic.scoping().symbol_declaration(symbol);
        let specifier = match self.semantic.nodes().kind(node) {
            AstKind::ImportSpecifier(s) => s,
            _ => return Origin::Unknown,
        };
        let Some(import) = self.semantic.nodes().ancestor_kinds(node).find_map(|kind| {
            if let AstKind::ImportDeclaration(i) = kind {
                Some(i)
            } else {
                None
            }
        }) else {
            return Origin::Unknown;
        };
        if !import.source.value.starts_with('.') {
            return Origin::Unknown;
        }
        let path = std::path::Path::new(filename)
            .parent()
            .unwrap_or(std::path::Path::new(""))
            .join(import.source.value.as_str());
        let target = normalize(&path.to_string_lossy());
        let Some((name, code)) = find_source(map, &target) else {
            return Origin::Unknown;
        };
        let allocator = Allocator::default();
        let source_type = SourceType::from_path(name)
            .unwrap_or_else(|_| SourceType::mjs().with_jsx(true))
            .with_module(true);
        let parsed = Parser::new(&allocator, code, source_type).parse();
        if parsed.fatal_error || parsed.diagnostics.has_errors() {
            return Origin::Unknown;
        }
        let semantic = SemanticBuilder::new()
            .with_build_nodes(true)
            .with_check_syntax_error(false)
            .build(&parsed.program);
        if semantic.semantic.nodes().len() > self.budget.get() || self.cancel.check().is_err() {
            return Origin::Unknown;
        }
        self.budget
            .set(self.budget.get() - semantic.semantic.nodes().len());
        let patterns = Patterns::new(
            &semantic.semantic,
            Some((map, name)),
            self.budget,
            self.cancel,
        );
        let exported = specifier.imported.name();
        let Some((&symbol, &function)) = patterns.functions.iter().find(|(s, _)| {
            patterns.semantic.scoping().symbol_name(**s) == exported.as_str()
                && patterns.exported(**s)
        }) else {
            return Origin::Unknown;
        };
        if patterns
            .semantic
            .scoping()
            .get_resolved_references(symbol)
            .any(|r| r.is_write())
        {
            return Origin::Unknown;
        }
        patterns.invoke(function, args, &Environment::new(), depth + 1)
    }
}

fn normalize(path: &str) -> String {
    let mut parts = vec![];
    for part in path.split('/') {
        match part {
            "." | "" => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn find_source<'a>(map: &'a sourcemap::DecodedMap, target: &str) -> Option<(&'a str, &'a str)> {
    let regular = match map {
        sourcemap::DecodedMap::Regular(map) => map,
        sourcemap::DecodedMap::Hermes(map) => map,
        sourcemap::DecodedMap::Index(map) => {
            return map
                .sections()
                .filter_map(|s| s.get_sourcemap())
                .find_map(|map| find_source(map, target));
        }
    };
    let mut result = None;
    for index in 0..regular.get_source_count() {
        let Some(name) = regular.get_source(index) else {
            continue;
        };
        let normalized = normalize(name);
        let matches = normalized == target
            || [".ts", ".tsx", ".js", ".jsx", "/index.ts", "/index.tsx"]
                .iter()
                .any(|suffix| normalized.strip_suffix(suffix) == Some(target));
        if matches {
            let code = regular.get_source_contents(index)?;
            if result.is_some_and(|(_, previous)| previous != code) {
                return None;
            }
            result = Some((name, code));
        }
    }
    result
}
