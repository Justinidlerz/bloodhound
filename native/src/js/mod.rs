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

mod bindings;
mod dependencies;
mod effects;
mod evaluation;
mod values;
mod visitor;
use values::*;

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
pub fn engine_version() -> String {
    "oxc-0.154.0".into()
}
pub fn analyze(source: String, options_json: String) -> napi::Result<String> {
    analyze_timed(
        source,
        options_json,
        &crate::timing::ResourceTimer::default(),
    )
}

pub(crate) fn analyze_timed(
    source: String,
    options_json: String,
    timer: &crate::timing::ResourceTimer,
) -> napi::Result<String> {
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
    let parsed = {
        let _parse = timer.task("jsParse");
        Parser::new(&allocator, &source, source_type).parse()
    };
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
    let semantic = {
        let _semantic = timer.task("jsSemantic");
        SemanticBuilder::new_compiler()
            .with_check_syntax_error(true)
            .build(&parsed.program)
    };
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
    let visit = timer.task("jsVisit");
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
    drop(visit);
    Ok(serde_json::to_string(&analyzer.output).unwrap())
}

#[cfg(test)]
#[path = "../tests/js.rs"]
mod tests;
