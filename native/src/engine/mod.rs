use crate::{js, markup, model::*, resolve, timing::Timings};
use base64::Engine as _;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs::{self, File},
    io::{BufWriter, Read, Write},
    path::Path,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use url::Url;
mod analysis;
mod loading;
mod queue;
mod report;
mod state;
use loading::*;
use report::date;
pub(crate) use report::{COLLECTIONS, validate, write_json};

struct Run {
    request: Request,
    budgets: Budgets,
    dependency_mode: String,
    config_hash: String,
    report: Value,
    queue: VecDeque<Work>,
    seen: HashMap<String, String>,
    cache: HashMap<String, Value>,
    cache_bytes: usize,
    snapshots: HashMap<String, (Work, Loaded, Value)>,
    flag: Arc<AtomicBool>,
    started: u64,
    deadline: u64,
    count: usize,
    bytes: usize,
    stream: Option<BufWriter<File>>,
    record_count: usize,
    timings: Timings,
}
pub fn scan(input: &str, flag: Arc<AtomicBool>) -> Result<String, String> {
    let timing_started = Instant::now();
    let mut request: Request = serde_json::from_str(input).map_err(|e| e.to_string())?;
    if request
        .options
        .get("timings")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err("invalid-timings-option".into());
    }
    let timings = Timings::new(request.options["timings"] == true, timing_started);
    let setup = timings.setup();
    if let Some(path) = request.options["resumePath"].as_str() {
        let checkpoint: Value = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        request.options["resume"] = checkpoint;
    }
    if request.options["resume"].is_object() {
        let cp = request.options["resume"].clone();
        if cp["checkpointVersion"] != 1 {
            return Err("unsupported-checkpoint-version".into());
        }
        request.entries =
            serde_json::from_value(cp["entries"].clone()).map_err(|e| e.to_string())?;
        for key in ["access", "resourceMappings", "sourceMaps"] {
            if request.options.get(key).is_none() {
                if let Some(value) = cp["options"].get(key) {
                    request.options[key] = value.clone();
                }
            }
        }
        for (k, v) in cp["report"]["configuration"]["context"]
            .as_object()
            .into_iter()
            .flatten()
        {
            if request.options.get(k).is_none() {
                request.options[k] = v.clone();
            }
        }
        for k in ["budgets", "dependencyMode"] {
            if request.options.get(k).is_none() {
                request.options[k] = cp["report"]["configuration"][k].clone();
            }
        }
    }
    if request.entries.is_empty() {
        return Err("at-least-one-entry-required".into());
    }
    let options = &request.options;
    let report_format = string(options, "reportFormat", "compact");
    if !["compact", "full"].contains(&report_format.as_str()) {
        return Err("invalid-report-format".into());
    }
    let mut budget_json = serde_json::to_value(Budgets::default()).unwrap();
    for (k, v) in options["budgets"].as_object().into_iter().flatten() {
        if budget_json.get(k).is_none() {
            return Err(format!("unknown-budget-{k}"));
        }
        budget_json[k] = v.clone();
    }
    let budgets: Budgets =
        serde_json::from_value(budget_json.clone()).map_err(|e| e.to_string())?;
    for (k, v) in budget_json.as_object().unwrap() {
        if !["retries", "maxDepth", "maxRedirects"].contains(&k.as_str()) && v.as_u64() == Some(0) {
            return Err(format!("invalid-budget-{k}"));
        }
    }
    let mut versions: BTreeMap<String, String> =
        serde_json::from_value(options.get("versions").cloned().unwrap_or(json!({})))
            .map_err(|e| e.to_string())?;
    versions.extend([
        ("scanner".into(), env!("CARGO_PKG_VERSION").into()),
        ("rules".into(), "1".into()),
        ("adapters".into(), "1".into()),
        ("parser".into(), "oxc-0.154.0".into()),
        ("configuration".into(), "1".into()),
    ]);
    let logical_entries: Vec<_> = request
        .entries
        .iter()
        .map(|e| {
            json!([
                e.kind,
                e.resource_url.clone().or_else(|| e.source.url().ok())
            ])
        })
        .collect();
    let context = Context {
        script_mode: string(options, "scriptMode", "unknown"),
        execution_environment: string(
            options,
            "executionEnvironment",
            if request.entries.iter().all(|e| e.kind == "html") {
                "window"
            } else {
                "unknown"
            },
        ),
        scope_key: options["scopeKey"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| hash(options.get("scope").unwrap_or(&json!(logical_entries)))),
        versions,
        base_url: options["baseUrl"].as_str().map(String::from),
        import_map: options.get("importMap").cloned(),
        import_map_base_url: options["importMapBaseUrl"].as_str().map(String::from),
        public_path: options["publicPath"].as_str().map(String::from),
    };
    if !["classic", "module", "unknown"].contains(&context.script_mode.as_str())
        || !["window", "worker", "unknown"].contains(&context.execution_environment.as_str())
    {
        return Err("invalid-execution-context".into());
    }
    let dependency_mode = string(options, "dependencyMode", "recursive");
    if !["single", "recursive"].contains(&dependency_mode.as_str()) {
        return Err("invalid-dependency-mode".into());
    }
    let config = json!({"context":context,"budgets":budgets,"dependencyMode":dependency_mode});
    let config_hash = hash(&config);
    if options["resume"].is_object() && options["resume"]["configHash"] != config_hash {
        return Err("checkpoint-configuration-mismatch".into());
    }
    let started = now();
    let report = json!({"schemaVersion":1,"indexSchemaVersion":1,"scannerVersion":env!("CARGO_PKG_VERSION"),"scanId":hash(&json!([request.entries,config,started])),"scopeKey":context.scope_key,"scanStatus":"completed","runStatus":"finished","startedAt":date(),"finishedAt":"","configuration":config,"entryAnalysisIds":[],"resources":[],"analysisInstances":[],"references":[],"globalTargets":[],"globalAccesses":[],"findings":[],"unresolved":[],"coverage":{"analyzed":0,"failed":0,"unresolved":0,"bytes":0,"dependencyMode":dependency_mode,"sourceMapped":0,"elapsedMs":0}});
    let mut queue = VecDeque::new();
    for e in &request.entries {
        if !["js", "html", "css"].contains(&e.kind.as_str()) {
            return Err("unsupported-entry-kind".into());
        }
        let url = e.resource_url.clone().unwrap_or(e.source.url()?);
        let u = Url::parse(&url).map_err(|_| "invalid-resource-url")?;
        if !u.username().is_empty() || u.password().is_some() {
            return Err("url-credentials-prohibited".into());
        }
        queue.push_back(Work {
            kind: e.kind.clone(),
            source: e.source.clone(),
            resource_url: url,
            context: context.clone(),
            depth: 0,
            incoming: vec![],
            inline: None,
        });
    }
    let stream = options["jsonlPath"]
        .as_str()
        .map(|path| {
            File::create(path)
                .map(BufWriter::new)
                .map_err(|e| e.to_string())
        })
        .transpose()?;
    let deadline = started + budgets.timeout_ms;
    let mut run = Run {
        request,
        budgets,
        dependency_mode,
        config_hash,
        report,
        queue,
        seen: HashMap::new(),
        cache: HashMap::new(),
        cache_bytes: 0,
        snapshots: HashMap::new(),
        flag,
        started,
        deadline,
        count: 0,
        bytes: 0,
        stream,
        record_count: 0,
        timings,
    };
    drop(setup);
    if let Err(e) = run.drain().and_then(|_| run.summarize()) {
        run.gap(&e, None, None, None)?;
    }
    let finalize = run.timings.stage("finalize");
    if let Err(reason) = stopped(&run.flag, run.deadline) {
        if reason == "cancelled" {
            run.report["runStatus"] = json!("cancelled");
        }
        run.gap(&reason, None, None, None)?;
    }
    let refs = run.report["references"].as_array().unwrap().clone();
    for mut r in refs {
        if r["resolution"] != "not-followed" && r["toAnalysisIds"].as_array().unwrap().is_empty() {
            r["resolution"] = json!("unresolved");
            if r.get("reason").is_none() {
                r["reason"] = json!("dependency-not-analyzed");
            }
            if !run.report["unresolved"]
                .as_array()
                .unwrap()
                .iter()
                .any(|u| u["referenceId"] == r["referenceId"])
            {
                run.gap(
                    text(&r["reason"]),
                    Some(text(&r["fromAnalysisId"])),
                    Some(text(&r["referenceId"])),
                    None,
                )?;
            }
        }
        if let Some(existing) = run.report["references"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|x| x["referenceId"] == r["referenceId"])
        {
            *existing = r.clone();
        }
        run.emit("references", &r)?;
    }
    let failed = run.report["analysisInstances"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["scanStatus"] == "failed")
        .count();
    let valid = run.report["analysisInstances"].as_array().unwrap().len() - failed;
    let unresolved = run.report["unresolved"].as_array().unwrap().len();
    run.report["scanStatus"] = json!(if valid == 0 {
        "failed"
    } else if unresolved > 0 || failed > 0 || !run.queue.is_empty() {
        "partial"
    } else {
        "completed"
    });
    run.report["finishedAt"] = json!(date());
    run.report["coverage"] = json!({"analyzed":valid,"failed":failed,"unresolved":unresolved,"bytes":run.bytes,"dependencyMode":run.dependency_mode,"sourceMapped":run.report["analysisInstances"].as_array().unwrap().iter().filter(|a|a["sourceMapStatus"]=="mapped").count(),"elapsedMs":now()-run.started});
    let analyzed_ids: HashSet<_> = run.report["analysisInstances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| text(&a["analysisId"]).to_string())
        .collect();
    run.report["entryAnalysisIds"]
        .as_array_mut()
        .unwrap()
        .retain(|id| analyzed_ids.contains(text(id)));
    {
        let _validation = run.timings.stage("validation");
        validate(&run.report)?;
    }
    run.checkpoint()?;
    let mut summary = run.report.clone();
    for key in COLLECTIONS {
        summary.as_object_mut().unwrap().remove(key);
    }
    let mut output = if report_format == "full" {
        std::mem::take(&mut run.report)
    } else {
        let _compact = run.timings.stage("compact");
        crate::report::compact(&run.report)
    };
    drop(finalize);
    if let Some(timings) = run.timings.finish() {
        output["timings"] = timings.clone();
        summary["timings"] = timings;
    }
    run.emit("summary", &summary)?;
    if let Some(stream) = &mut run.stream {
        stream.flush().map_err(|e| e.to_string())?;
    }
    if let Some(path) = run.request.options["reportPath"].as_str() {
        write_json(path, &output)?;
    }
    Ok(output.to_string())
}

#[cfg(test)]
#[path = "../tests/engine.rs"]
mod tests;
