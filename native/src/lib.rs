mod bridge;
mod engine;
mod js;
mod markup;
mod model;
mod query;
mod report;
mod resolve;
mod timing;
use bridge::{CheckpointInput, JsonOutput, ReportInput};
use napi::{
    Env, ScopedTask,
    bindgen_prelude::{AsyncTask, FnArgs, Function, FunctionRef, Object, ObjectRef, Unknown},
};
use napi_derive::napi;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

static RUNS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);
pub(crate) fn cancellation(id: &str) -> Arc<AtomicBool> {
    RUNS.get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(id.into())
        .or_default()
        .clone()
}
pub struct ScanTask {
    request: String,
    id: String,
    flag: Arc<AtomicBool>,
    listener: Option<(ObjectRef, FunctionRef<(), ()>)>,
}
impl Drop for ScanTask {
    fn drop(&mut self) {
        RUNS.get_or_init(Default::default)
            .lock()
            .unwrap()
            .remove(&self.id);
    }
}
impl<'task> ScopedTask<'task> for ScanTask {
    type Output = Value;
    type JsValue = Unknown<'task>;
    fn compute(&mut self) -> napi::Result<Value> {
        let result =
            engine::scan(&self.request, self.flag.clone()).map_err(napi::Error::from_reason)?;
        serde_json::from_str(&result).map_err(|e| napi::Error::from_reason(e.to_string()))
    }
    fn resolve(&mut self, env: &'task Env, value: Value) -> napi::Result<Unknown<'task>> {
        env.to_js_value(&value)
    }
    fn finally(mut self, env: Env) -> napi::Result<()> {
        if let Some((signal, callback)) = self.listener.take() {
            let result = (|| {
                let object = signal.get_value(&env)?;
                let remove = object
                    .get::<Function<FnArgs<(String, Function<(), ()>)>, ()>>("removeEventListener")?
                    .ok_or_else(|| {
                        napi::Error::from_reason("signal-remove-event-listener-required")
                    })?;
                remove.apply(object, ("abort".into(), callback.borrow_back(&env)?).into())
            })();
            let unref = signal.unref(&env);
            result.and(unref)?;
        }
        Ok(())
    }
}

fn start(
    env: Env,
    entries: Value,
    options: Option<Object>,
    overrides: Value,
) -> napi::Result<AsyncTask<ScanTask>> {
    let id = NEXT_RUN.fetch_add(1, Ordering::Relaxed).to_string();
    let mut config = json!({});
    let mut listener = None;
    let flag = Arc::new(AtomicBool::new(false));
    if let Some(options) = options {
        for key in Object::keys(&options)? {
            if key == "resume" {
                if let Some(value) = options.get::<CheckpointInput>(&key)? {
                    config[key] = value.0;
                }
            } else if key != "signal" {
                if let Some(value) = options.get::<Value>(&key)? {
                    config[&key] = value;
                }
            }
        }
        if let Some(object) = options.get::<Object>("signal")? {
            flag.store(
                object.get::<bool>("aborted")?.unwrap_or(false),
                Ordering::Relaxed,
            );
            let cancelled = flag.clone();
            let callback =
                env.create_function_from_closure::<(), (), _>("cancelScan", move |_| {
                    cancelled.store(true, Ordering::Relaxed);
                    Ok(())
                })?;
            let add = object
                .get::<Function<FnArgs<(String, Function<(), ()>)>, ()>>("addEventListener")?
                .ok_or_else(|| napi::Error::from_reason("signal-add-event-listener-required"))?;
            let callback_ref = callback.create_ref()?;
            let signal_ref = object.create_ref()?;
            if let Err(error) = add.apply(object, ("abort".into(), callback).into()) {
                signal_ref.unref(&env)?;
                return Err(error);
            }
            listener = Some((signal_ref, callback_ref));
        }
    }
    for (key, value) in overrides.as_object().into_iter().flatten() {
        config[key] = value.clone();
    }
    config["runId"] = json!(id);
    RUNS.get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(id.clone(), flag.clone());
    Ok(AsyncTask::new(ScanTask {
        request: json!({"entries":entries,"options":config}).to_string(),
        id,
        flag,
        listener,
    }))
}
#[napi(
    ts_args_type = "entries: ScanEntry[], options?: T",
    ts_generic_types = "T extends ScanOptions = {}",
    ts_return_type = "Promise<ScanResult<T>>"
)]
pub fn scan(
    env: Env,
    entries: Value,
    options: Option<Object>,
) -> napi::Result<AsyncTask<ScanTask>> {
    start(env, entries, options, json!({}))
}
fn source(input: Value) -> Value {
    match input.as_str() {
        Some(value) => json!({"type":"content","content":value}),
        None => input,
    }
}
fn single_entry(
    env: Env,
    kind: &str,
    input: Value,
    options: Option<Object>,
) -> napi::Result<AsyncTask<ScanTask>> {
    let resource_url = options
        .as_ref()
        .map(|options| options.get::<String>("resourceUrl"))
        .transpose()?
        .flatten();
    start(
        env,
        json!([{"kind":kind,"source":source(input),"resourceUrl":resource_url}]),
        options,
        json!({}),
    )
}
#[napi(
    ts_args_type = "input: Source | string, options?: T",
    ts_generic_types = "T extends ScanOptions = {}",
    ts_return_type = "Promise<ScanResult<T>>"
)]
pub fn scan_js(
    env: Env,
    input: Value,
    options: Option<Object>,
) -> napi::Result<AsyncTask<ScanTask>> {
    single_entry(env, "js", input, options)
}
#[napi(
    ts_args_type = "input: Source | string, options?: T",
    ts_generic_types = "T extends ScanOptions = {}",
    ts_return_type = "Promise<ScanResult<T>>"
)]
pub fn scan_html(
    env: Env,
    input: Value,
    options: Option<Object>,
) -> napi::Result<AsyncTask<ScanTask>> {
    single_entry(env, "html", input, options)
}
#[napi(
    ts_args_type = "entry: ScanEntry, options?: T",
    ts_generic_types = "T extends ScanOptions = {}",
    ts_return_type = "Promise<ScanResult<T>>"
)]
pub fn scan_resource(
    env: Env,
    entry: Value,
    options: Option<Object>,
) -> napi::Result<AsyncTask<ScanTask>> {
    start(
        env,
        json!([entry]),
        options,
        json!({"dependencyMode":"single"}),
    )
}
#[napi(
    ts_args_type = "checkpoint: Checkpoint, options?: T",
    ts_generic_types = "T extends ScanOptions = {}",
    ts_return_type = "Promise<ScanResult<T>>"
)]
pub fn resume_scan(
    env: Env,
    checkpoint: CheckpointInput,
    options: Option<Object>,
) -> napi::Result<AsyncTask<ScanTask>> {
    start(env, json!([]), options, json!({"resume":checkpoint.0}))
}
#[napi(
    ts_args_type = "report: unknown",
    ts_return_type = "asserts report is Report"
)]
pub fn validate_report(report: ReportInput) -> napi::Result<()> {
    engine::validate(&report.0).map_err(napi::Error::from_reason)
}
#[napi(ts_args_type = "report: Report", ts_return_type = "Manifest")]
pub fn compact_report(report: ReportInput) -> napi::Result<JsonOutput> {
    engine::validate(&report.0).map_err(napi::Error::from_reason)?;
    Ok(JsonOutput(report::compact(&report.0)))
}
#[napi]
pub fn engine_version() -> String {
    js::engine_version()
}

#[napi]
pub struct GlobalIndex {
    inner: query::Index,
}
#[napi]
impl GlobalIndex {
    #[napi(constructor, ts_args_type = "report: Report")]
    pub fn new(report: ReportInput) -> napi::Result<Self> {
        Ok(Self {
            inner: query::Index::new(report.0).map_err(napi::Error::from_reason)?,
        })
    }
    #[napi(getter, ts_return_type = "Report")]
    pub fn report(&self) -> JsonOutput {
        JsonOutput(self.inner.report.clone())
    }
    #[napi(ts_return_type = "Target[]")]
    pub fn find_targets(
        &self,
        path_segments: Vec<String>,
        namespace: Option<String>,
        scope_key: Option<String>,
    ) -> napi::Result<JsonOutput> {
        let mut args = json!({"pathSegments":path_segments});
        if let Some(namespace) = namespace {
            args["namespace"] = json!(namespace);
        }
        if let Some(scope_key) = scope_key {
            args["scopeKey"] = json!(scope_key);
        }
        self.invoke("findTargets", args)
    }
    #[napi(
        ts_return_type = "{ access: GlobalAccess; analysis: AnalysisInstance; resource: Resource }[]"
    )]
    pub fn sources(&self, target_id: String) -> napi::Result<JsonOutput> {
        self.invoke("sources", json!({"id":target_id}))
    }
    #[napi(ts_return_type = "GlobalAccess[]")]
    pub fn accesses(&self, analysis_id: String) -> napi::Result<JsonOutput> {
        self.invoke("accesses", json!({"id":analysis_id}))
    }
    #[napi(ts_return_type = "GlobalAccess[]")]
    pub fn resource_accesses(&self, resource_id: String) -> napi::Result<JsonOutput> {
        self.invoke("resourceAccesses", json!({"id":resource_id}))
    }
    #[napi(
        ts_args_type = "targetId: string, options?: { limit?: number; offset?: number; maxVisits?: number }",
        ts_return_type = "{ paths: { entryAnalysisId: string; analysisIds: string[]; referenceIds: string[] }[]; truncated: boolean; nextOffset: number }"
    )]
    pub fn entry_paths(
        &self,
        target_id: String,
        options: Option<Value>,
    ) -> napi::Result<JsonOutput> {
        let mut args = options.unwrap_or(json!({}));
        if !args.is_object() {
            return Err(napi::Error::from_reason(
                "entry-path-options-must-be-object",
            ));
        }
        args["id"] = json!(target_id);
        self.invoke("entryPaths", args)
    }
    #[napi(
        ts_return_type = "{ targetId: string; accessIds: string[]; analysisIds: string[]; valueTypes: string[]; certainty: 'static-candidate' }[]"
    )]
    pub fn conflicts(&self) -> napi::Result<JsonOutput> {
        self.invoke("conflicts", json!({}))
    }
    #[napi(
        ts_return_type = "{ targetId: string; candidates: string[]; certainty: 'static-candidate' }[]"
    )]
    pub fn resolve_unbound(&self) -> napi::Result<JsonOutput> {
        self.invoke("resolveUnbound", json!({}))
    }
}
impl GlobalIndex {
    fn invoke(&self, operation: &str, args: Value) -> napi::Result<JsonOutput> {
        self.inner
            .invoke(operation, &args)
            .map(JsonOutput)
            .map_err(napi::Error::from_reason)
    }
}
fn report_query(report: ReportInput, operation: &str, args: Value) -> napi::Result<JsonOutput> {
    query::Index::new(report.0)
        .and_then(|index| index.invoke(operation, &args))
        .map(JsonOutput)
        .map_err(napi::Error::from_reason)
}
#[napi(
    ts_args_type = "before: Report, after: Report",
    ts_return_type = "{ added: GlobalAccess[]; removed: GlobalAccess[]; changed: { before: GlobalAccess; after: GlobalAccess }[] }"
)]
pub fn diff_reports(before: ReportInput, after: ReportInput) -> napi::Result<JsonOutput> {
    report_query(before, "diffReports", json!({"after":after.0}))
}
#[napi(
    ts_args_type = "report: Report, reportId?: string",
    ts_return_type = "Pick<Report, 'indexSchemaVersion' | 'scopeKey' | 'entryAnalysisIds' | 'resources' | 'analysisInstances' | 'references' | 'globalTargets' | 'globalAccesses' | 'findings' | 'unresolved'> & { reportId: string }"
)]
pub fn index_artifact(report: ReportInput, report_id: Option<String>) -> napi::Result<JsonOutput> {
    let id = report_id
        .map(Value::String)
        .unwrap_or_else(|| report.0["scanId"].clone());
    report_query(report, "indexArtifact", json!({"reportId":id}))
}
#[napi(
    ts_args_type = "report: Report, type: 'build' | 'page', metadata?: Record<string, unknown>",
    ts_return_type = "Record<string, unknown>"
)]
pub fn adapt_report(
    report: ReportInput,
    kind: String,
    metadata: Option<Value>,
) -> napi::Result<JsonOutput> {
    report_query(
        report,
        "adaptReport",
        json!({"type":kind,"metadata":metadata.unwrap_or(json!({}))}),
    )
}
#[napi(
    ts_args_type = "report: Report, input: { resourceUrl?: string; contentHash?: string; line?: number; column?: number; pathSegments?: string[] }",
    ts_return_type = "{ accessId: string; attribution: 'static_candidate' }[]"
)]
pub fn runtime_candidates(report: ReportInput, input: Value) -> napi::Result<JsonOutput> {
    report_query(report, "runtimeCandidates", input)
}
#[napi(ts_return_type = "{ budgets: Budgets; versions: Record<string, string> }")]
pub fn defaults() -> Value {
    json!({"budgets":model::Budgets::default(),"versions":{"scanner":env!("CARGO_PKG_VERSION"),"rules":"1","adapters":"1","parser":"oxc-0.154.0","configuration":"1"}})
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
#[path = "tests/bindings.rs"]
mod tests;
