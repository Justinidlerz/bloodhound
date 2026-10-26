//! Node.js bindings.
//!
//! Three entry points are exported: `scan` (any entries), `scanHtml` and
//! `scanJs` (one entry, string or source object). All of them run the scan
//! on the libuv thread pool and resolve with a `Manifest`. An optional
//! `AbortSignal` cancels a running scan cooperatively.
//!
//! The JSON boundary is crossed exactly twice per scan: the request comes
//! in as a JSON string and the manifest goes out as a JSON string, which
//! `index.js` parses. This keeps the binding tiny and lets the Rust side own
//! every data structure.

mod html;
mod js;
mod lines;
mod loader;
mod model;
mod report;
mod resolve;
mod scheduler;

use napi::bindgen_prelude::{AsyncTask, FnArgs, Function, FunctionRef, Object, ObjectRef};
use napi::{Env, Task};
use napi_derive::napi;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Runs one scan on a background thread.
pub struct ScanTask {
    request: String,
    flag: Arc<AtomicBool>,
    /// `(signal, listener)` to detach when the task finishes.
    listener: Option<(ObjectRef, FunctionRef<(), ()>)>,
}

impl Task for ScanTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> napi::Result<String> {
        scan_json(&self.request, self.flag.clone()).map_err(napi::Error::from_reason)
    }

    fn resolve(&mut self, _env: Env, output: String) -> napi::Result<String> {
        Ok(output)
    }

    fn finally(mut self, env: Env) -> napi::Result<()> {
        if let Some((signal, callback)) = self.listener.take() {
            let detach = (|| {
                let object = signal.get_value(&env)?;
                let remove = object
                    .get::<Function<FnArgs<(String, Function<(), ()>)>, ()>>("removeEventListener")?
                    .ok_or_else(|| {
                        napi::Error::from_reason("signal-remove-event-listener-required")
                    })?;
                remove.apply(object, ("abort".into(), callback.borrow_back(&env)?).into())
            })();
            let unref = signal.unref(&env);
            detach.and(unref)?;
        }
        Ok(())
    }
}

/// Parses the request, runs the scheduler, and serializes the manifest.
pub(crate) fn scan_json(request: &str, flag: Arc<AtomicBool>) -> Result<String, String> {
    let request: model::Request =
        serde_json::from_str(request).map_err(|e| format!("invalid-request: {e}"))?;
    let graph = scheduler::run(&request, flag)?;
    let manifest = report::build(graph, request.options.dependency_mode);
    serde_json::to_string(&manifest).map_err(|e| e.to_string())
}

/// Builds the task from JS arguments. `options.signal` is detached from the
/// JSON payload and wired to the cancellation flag.
fn start(env: Env, entries: Value, options: Option<Object>) -> napi::Result<AsyncTask<ScanTask>> {
    let flag = Arc::new(AtomicBool::new(false));
    let mut config = json!({});
    let mut listener = None;
    if let Some(options) = options {
        for key in Object::keys(&options)? {
            if key == "signal" {
                continue;
            }
            if let Some(value) = options.get::<Value>(&key)? {
                config[&key] = value;
            }
        }
        if let Some(signal) = options.get::<Object>("signal")? {
            flag.store(
                signal.get::<bool>("aborted")?.unwrap_or(false),
                Ordering::Relaxed,
            );
            let cancelled = flag.clone();
            let callback =
                env.create_function_from_closure::<(), (), _>("cancelScan", move |_| {
                    cancelled.store(true, Ordering::Relaxed);
                    Ok(())
                })?;
            let add = signal
                .get::<Function<FnArgs<(String, Function<(), ()>)>, ()>>("addEventListener")?
                .ok_or_else(|| napi::Error::from_reason("signal-add-event-listener-required"))?;
            let callback_ref = callback.create_ref()?;
            let signal_ref = signal.create_ref()?;
            if let Err(error) = add.apply(signal, ("abort".into(), callback).into()) {
                signal_ref.unref(&env)?;
                return Err(error);
            }
            listener = Some((signal_ref, callback_ref));
        }
    }
    let request = json!({ "entries": entries, "options": config }).to_string();
    Ok(AsyncTask::new(ScanTask {
        request,
        flag,
        listener,
    }))
}

/// Normalizes a string input to a content source.
fn source_of(input: Value) -> Value {
    match input.as_str() {
        Some(text) => json!({ "type": "content", "content": text }),
        None => input,
    }
}

/// Scans a list of entries.
#[napi(
    js_name = "scanRaw",
    ts_args_type = "entries: ScanEntry[], options?: ScanOptions",
    ts_return_type = "Promise<string>"
)]
pub fn scan_raw(
    env: Env,
    entries: Value,
    options: Option<Object>,
) -> napi::Result<AsyncTask<ScanTask>> {
    start(env, entries, options)
}

/// Scans one HTML document given as text or a source object.
#[napi(
    js_name = "scanHtmlRaw",
    ts_args_type = "input: Source | string, options?: ScanOptions",
    ts_return_type = "Promise<string>"
)]
pub fn scan_html_raw(
    env: Env,
    input: Value,
    options: Option<Object>,
) -> napi::Result<AsyncTask<ScanTask>> {
    let resource_url = options
        .as_ref()
        .map(|o| o.get::<String>("resourceUrl"))
        .transpose()?
        .flatten();
    let entry = json!({ "kind": "html", "source": source_of(input), "resourceUrl": resource_url });
    start(env, json!([entry]), options)
}

/// Scans one JavaScript resource given as text or a source object.
#[napi(
    js_name = "scanJsRaw",
    ts_args_type = "input: Source | string, options?: ScanOptions",
    ts_return_type = "Promise<string>"
)]
pub fn scan_js_raw(
    env: Env,
    input: Value,
    options: Option<Object>,
) -> napi::Result<AsyncTask<ScanTask>> {
    let resource_url = options
        .as_ref()
        .map(|o| o.get::<String>("resourceUrl"))
        .transpose()?
        .flatten();
    let entry = json!({ "kind": "js", "source": source_of(input), "resourceUrl": resource_url });
    start(env, json!([entry]), options)
}

/// Scanner and parser versions.
#[napi(ts_return_type = "{ scanner: string; parser: string; schema: number }")]
pub fn version() -> Value {
    json!({ "scanner": env!("CARGO_PKG_VERSION"), "parser": js::PARSER_VERSION, "schema": 2 })
}

/// Default budgets, for documentation and plugin defaults.
#[napi(ts_return_type = "Budgets")]
pub fn default_budgets() -> Value {
    serde_json::to_value(model::Budgets::default()).unwrap_or(Value::Null)
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
#[path = "tests/bindings.rs"]
mod tests;
