use super::*;

impl<'s, 'a> Analyzer<'s, 'a> {
    pub(super) fn expression_key(&self, e: &Expression<'a>) -> Option<String> {
        match e {
            Expression::Identifier(i) => self.symbol(i).map(|s| format!("s{s:?}")),
            Expression::StaticMemberExpression(m) => self
                .expression_key(&m.object)
                .map(|s| format!("{s}.{}", m.property.name)),
            _ => None,
        }
    }
    pub(super) fn register_function(&mut self, key: String, e: &Expression<'a>) {
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
    pub(super) fn bind(&mut self, p: &BindingPattern<'a>, value: Value) {
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
    pub(super) fn invalidate_mutable(&mut self) {
        self.values.retain(|id, v| {
            self.scoping.symbol_flags(*id).is_const_variable()
                && !matches!(v, Value::Object(_) | Value::Array(_))
        });
    }
    pub(super) fn inspect_variable_declarator(&mut self, v: &VariableDeclarator<'a>) {
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
    pub(super) fn inspect_binding_identifier(&mut self, i: &BindingIdentifier<'a>) {
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
    pub(super) fn inspect_import_declaration(&mut self, i: &ImportDeclaration<'a>) {
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
    pub(super) fn inspect_export_named_declaration(&mut self, e: &ExportNamedDeclaration<'a>) {
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
    pub(super) fn inspect_export_from_declaration(&mut self, e: &ExportFromDeclaration<'a>) {
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
    pub(super) fn inspect_export_declaration(&mut self, e: &ExportDeclaration<'a>) {
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
    pub(super) fn inspect_export_default_declaration(&mut self, e: &ExportDefaultDeclaration<'a>) {
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
    pub(super) fn inspect_export_all_declaration(&mut self, e: &ExportAllDeclaration<'a>) {
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
    pub(super) fn inspect_import_expression(&mut self, i: &ImportExpression<'a>) {
        let v = self.eval(&i.source);
        self.dep(v, i.span, "dynamic-import", "module", "inherit");
        walk::walk_import_expression(self, i);
    }
}
