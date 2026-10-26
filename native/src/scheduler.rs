//! Scan scheduling.
//!
//! Architecture:
//!
//! ```text
//!              ┌──────────────┐  Job   ┌──────────────┐
//!   entries ──▶│ Coordinator  │───────▶│ Worker pool  │
//!              │ (graph owner)│◀───────│ load+analyze │
//!              └──────────────┘ Result └──────────────┘
//! ```
//!
//! The coordinator runs on the calling thread and is the only owner of the
//! resource graph. It hands `Job`s to a fixed pool of worker threads over a
//! channel; each worker loads one resource and runs the analyzer for its
//! kind, then sends back a `Done` message. The coordinator records the
//! result, resolves the dependencies it discovered, and enqueues children
//! that have not been seen. The loop ends when no jobs are in flight and the
//! queue is empty, or when a budget is exhausted.
//!
//! Resource identity includes the document context: the same URL may resolve
//! differently under two import maps. Analysis is never reused across URLs;
//! `import.meta.url` and document-relative loaders depend on that context.

use crate::html;
use crate::js::{self, JsOptions};
use crate::loader::{self, Loaded, Loader};
use crate::model::*;
use crate::resolve;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Cooperative cancellation shared by every thread of one scan.
#[derive(Clone)]
pub struct Cancellation {
    flag: Arc<AtomicBool>,
    deadline: Instant,
}

impl Cancellation {
    pub fn new(flag: Arc<AtomicBool>, timeout: Duration) -> Self {
        Self {
            flag,
            deadline: Instant::now() + timeout,
        }
    }

    /// A cancellation that never fires. Used by tests and standalone analysis.
    #[allow(dead_code)]
    pub fn never() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() + Duration::from_secs(3600),
        }
    }

    #[allow(dead_code)]
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// `Err("cancelled")` or `Err("time-budget")` when the scan must stop.
    pub fn check(&self) -> Result<(), &'static str> {
        if self.flag.load(Ordering::Relaxed) {
            Err("cancelled")
        } else if Instant::now() >= self.deadline {
            Err("time-budget")
        } else {
            Ok(())
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}

/// Unique key of a resource in the graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ResourceKey {
    kind: Kind,
    url: String,
    mode: Option<ScriptMode>,
    context: usize,
}

/// A unit of work handed to a worker.
struct Job {
    index: usize,
    kind: Kind,
    source: Source,
    url: String,
    mode: Option<ScriptMode>,
    /// Document base for inline scripts (workers resolve against it).
    base_url: Option<String>,
}

/// Outcome of a job.
struct Done {
    index: usize,
    result: Result<(Loaded, Analysis, Option<sourcemap::DecodedMap>), String>,
}

/// A node in the resource graph, owned by the coordinator.
pub struct Resource {
    context: usize,
    /// Document base inherited by DOM-created resources through module imports.
    document_base: Option<String>,
    pub key_kind: Kind,
    pub url: String,
    pub mode: Option<ScriptMode>,
    pub depth: usize,
    pub is_entry: bool,
    /// Parent resource for inline content, with the byte offset.
    pub inline: Option<(usize, u32)>,
    /// Indices of resources that referenced this one.
    pub referrers: Vec<usize>,
    pub loaded: Option<Loaded>,
    pub analysis: Option<Analysis>,
    pub source_map: Option<sourcemap::DecodedMap>,
    pub status: SourceStatus,
    /// Load/parse failure reason.
    pub failure: Option<String>,
    /// Resolved dependency edges: (dependency index in analysis, target, status, reason).
    pub edges: Vec<Edge>,
    /// Import map in effect for this resource's module children.
    pub import_map: Option<ImportMap>,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub dependency: usize,
    pub to: Option<usize>,
    pub status: DependencyStatus,
    pub reason: Option<String>,
    pub url: Option<String>,
}

/// Everything the report builder needs.
pub struct Graph {
    pub resources: Vec<Resource>,
    pub entries: Vec<usize>,
    /// Scan-level gaps not attributable to one resource.
    pub gaps: Vec<(Option<usize>, Gap)>,
    pub cancelled: bool,
    pub bytes: usize,
    pub elapsed_ms: u64,
}

/// Compiles `excludeUrls` regex patterns once per scan.
fn compile_exclude(patterns: &[String]) -> Result<regex::RegexSet, String> {
    if patterns.iter().any(|p| p.is_empty()) {
        return Err("invalid-exclude-url-pattern: empty pattern".into());
    }
    regex::RegexSet::new(patterns).map_err(|e| format!("invalid-exclude-url-pattern: {e}"))
}

/// Runs a scan and returns the resource graph.
pub fn run(request: &Request, flag: Arc<AtomicBool>) -> Result<Graph, String> {
    let started = Instant::now();
    let options = &request.options;
    validate(request)?;
    let exclude = compile_exclude(&options.exclude_urls)?;
    let cancel = Cancellation::new(flag, Duration::from_millis(options.budgets.timeout_ms));
    let loader = Arc::new(Loader::new(options)?);

    let mut coordinator = Coordinator::new(options, cancel.clone(), exclude);
    for entry in &request.entries {
        let url = match &entry.resource_url {
            Some(u) => u.clone(),
            None => entry.source.url()?,
        };
        resolve::reject_credentials(&url)?;
        // Excluding an entry would silently produce an empty result, so a
        // matching entry is a configuration error rather than a skip.
        if coordinator.exclude.is_match(&url) {
            return Err(format!("entry-excluded: {url}"));
        }
        let mode = match entry.kind {
            Kind::Js => Some(entry.script_mode.unwrap_or(options.script_mode)),
            _ => None,
        };
        let index = coordinator.add(entry.kind, url.clone(), mode, 0, None, None);
        coordinator.resources[index].is_entry = true;
        coordinator.entries.push(index);
        coordinator.enqueue(index, entry.source.clone(), options.base_url.clone(), 0);
    }

    let pool = Pool::new(
        options
            .budgets
            .analysis_concurrency
            .min(options.budgets.concurrency),
        loader,
        options.clone(),
        cancel.clone(),
    );
    coordinator.drive(&pool);
    drop(pool);

    let cancelled = cancel.is_cancelled();
    Ok(Graph {
        bytes: coordinator.bytes,
        resources: coordinator.resources,
        entries: coordinator.entries,
        gaps: coordinator.gaps,
        cancelled,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

fn validate(request: &Request) -> Result<(), String> {
    if request.entries.is_empty() {
        return Err("at-least-one-entry-required".into());
    }
    let b = &request.options.budgets;
    for (name, value) in [
        ("concurrency", b.concurrency),
        ("analysisConcurrency", b.analysis_concurrency),
        ("maxResources", b.max_resources),
        ("maxResourceBytes", b.max_resource_bytes),
        ("maxBytes", b.max_bytes),
        ("maxSteps", b.max_steps),
        ("maxCandidates", b.max_candidates),
    ] {
        if value == 0 {
            return Err(format!("invalid-budget-{name}"));
        }
    }
    if b.timeout_ms == 0 || b.resource_timeout_ms == 0 {
        return Err("invalid-budget-timeoutMs".into());
    }
    if request.entries.len() > b.max_resources {
        return Err("entry-count-exceeds-resource-budget".into());
    }
    if b.concurrency > 256 || b.analysis_concurrency > 256 {
        return Err("concurrency-exceeds-256".into());
    }
    if let Some(map) = &request.options.import_map {
        let base = request
            .options
            .base_url
            .as_deref()
            .unwrap_or("memory:import-map");
        resolve::parse_import_map(map, base)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Worker pool
// ---------------------------------------------------------------------------

struct Pool {
    jobs: Option<Sender<Job>>,
    results: Receiver<Done>,
    handles: Vec<std::thread::JoinHandle<()>>,
}

impl Pool {
    fn new(size: usize, loader: Arc<Loader>, options: Options, cancel: Cancellation) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done_rx) = mpsc::channel::<Done>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let handles = (0..size)
            .map(|_| {
                let job_rx = Arc::clone(&job_rx);
                let done_tx = done_tx.clone();
                let loader = Arc::clone(&loader);
                let options = options.clone();
                let cancel = cancel.clone();
                std::thread::spawn(move || {
                    loop {
                        let job = match job_rx.lock() {
                            Ok(rx) => rx.recv(),
                            Err(_) => return,
                        };
                        let Ok(job) = job else { return };
                        let index = job.index;
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            work(job, &loader, &options, &cancel)
                        }))
                        .unwrap_or_else(|_| Err("analysis-worker-panicked".into()));
                        if done_tx.send(Done { index, result }).is_err() {
                            return;
                        }
                    }
                })
            })
            .collect();
        Self {
            jobs: Some(job_tx),
            results: done_rx,
            handles,
        }
    }

    fn submit(&self, job: Job) {
        if let Some(tx) = &self.jobs {
            let _ = tx.send(job);
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.jobs.take();
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}

/// Loads and analyzes one resource on a worker thread.
fn work(
    job: Job,
    loader: &Loader,
    options: &Options,
    cancel: &Cancellation,
) -> Result<(Loaded, Analysis, Option<sourcemap::DecodedMap>), String> {
    let mut loaded = loader.load(&job.source, cancel)?;
    let url = if job.url != job.source.url()? || matches!(job.source, Source::Content { .. }) {
        job.url.clone()
    } else {
        loaded.final_url.clone()
    };
    loaded.final_url = url.clone();
    let analysis = match job.kind {
        Kind::Html => html::extract(
            &loaded.content,
            if url.starts_with("memory:") {
                job.base_url.as_deref().unwrap_or(&url)
            } else {
                &url
            },
            options.budgets.max_steps,
        ),
        Kind::Js => {
            let mode = job.mode.unwrap_or(options.script_mode);
            let js_options = JsOptions {
                script_mode: mode,
                resource_url: url.clone(),
                base_url: job.base_url.clone().or_else(|| Some(url.clone())),
                max_steps: options.budgets.max_steps,
                max_candidates: options.budgets.max_candidates,
            };
            js::analyze(&loaded.content, &js_options, cancel)
        }
        Kind::Css => Analysis::default(),
    };
    let map = if options.source_maps && job.kind == Kind::Js {
        loader::source_map(loader, &loaded.content, &loaded.final_url, cancel)
    } else {
        None
    };
    Ok((loaded, analysis, map))
}

// ---------------------------------------------------------------------------
// Coordinator
// ---------------------------------------------------------------------------

struct Coordinator<'o> {
    options: &'o Options,
    exclude: regex::RegexSet,
    cancel: Cancellation,
    resources: Vec<Resource>,
    by_key: HashMap<ResourceKey, usize>,
    entries: Vec<usize>,
    /// (resource index, source to load, document base).
    queue: VecDeque<(usize, Source, Option<String>)>,
    in_flight: HashSet<usize>,
    gaps: Vec<(Option<usize>, Gap)>,
    bytes: usize,
    loaded_count: usize,
    stopped: Option<&'static str>,
}

impl<'o> Coordinator<'o> {
    fn new(options: &'o Options, cancel: Cancellation, exclude: regex::RegexSet) -> Self {
        Self {
            options,
            exclude,
            cancel,
            resources: vec![],
            by_key: HashMap::new(),
            entries: vec![],
            queue: VecDeque::new(),
            in_flight: HashSet::new(),
            gaps: vec![],
            bytes: 0,
            loaded_count: 0,
            stopped: None,
        }
    }

    /// Adds a resource node (or returns the existing one for the same key).
    fn add(
        &mut self,
        kind: Kind,
        url: String,
        mode: Option<ScriptMode>,
        depth: usize,
        referrer: Option<usize>,
        inline: Option<(usize, u32)>,
    ) -> usize {
        let context = referrer
            .map(|p| {
                if self.resources[p].key_kind == Kind::Html {
                    p + 1
                } else {
                    self.resources[p].context
                }
            })
            .unwrap_or(0);
        let key = ResourceKey {
            kind,
            url: url.clone(),
            mode,
            context,
        };
        if let Some(&index) = self.by_key.get(&key) {
            if let Some(r) = referrer {
                if !self.resources[index].referrers.contains(&r) {
                    self.resources[index].referrers.push(r);
                }
            }
            return index;
        }
        let index = self.resources.len();
        self.resources.push(Resource {
            context,
            document_base: None,
            key_kind: kind,
            url,
            mode,
            depth,
            is_entry: false,
            inline,
            referrers: referrer.into_iter().collect(),
            loaded: None,
            analysis: None,
            source_map: None,
            status: SourceStatus::Failed,
            failure: None,
            edges: vec![],
            import_map: None,
        });
        self.by_key.insert(key, index);
        index
    }

    fn stop(&mut self, reason: &'static str) {
        if self.stopped.is_none() {
            self.stopped = Some(reason);
            self.gaps.push((
                None,
                Gap {
                    reason: reason.into(),
                    span: Span::default(),
                    detail: None,
                },
            ));
        }
    }

    /// Main loop: submit queued jobs, consume results, repeat until idle.
    fn drive(&mut self, pool: &Pool) {
        let max_in_flight = self.options.budgets.concurrency.max(1);
        loop {
            if self.stopped.is_none() {
                if let Err(reason) = self.cancel.check() {
                    self.stop(reason);
                }
            }
            // Submit while there is capacity and no stop condition.
            while self.stopped.is_none() && self.in_flight.len() < max_in_flight {
                let Some((index, source, base_url)) = self.queue.pop_front() else {
                    break;
                };
                if self.loaded_count >= self.options.budgets.max_resources {
                    self.queue.push_front((index, source, base_url));
                    self.stop("resource-count-budget");
                    break;
                }
                self.loaded_count += 1;
                self.in_flight.insert(index);
                let r = &self.resources[index];
                pool.submit(Job {
                    index,
                    kind: r.key_kind,
                    source,
                    url: r.url.clone(),
                    mode: r.mode,
                    base_url,
                });
            }
            if self.in_flight.is_empty() {
                break;
            }
            // Wait for one result. A bounded wait keeps cancellation
            // responsive even if a worker is stuck in a slow network read.
            match pool.results.recv_timeout(Duration::from_millis(200)) {
                Ok(done) => {
                    self.in_flight.remove(&done.index);
                    self.record(done);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        // Anything still queued when we stopped is reported as unresolved by
        // the report builder (edges point to a resource with no analysis).
        for (index, _, _) in self.queue.drain(..) {
            self.resources[index].failure = Some(self.stopped.unwrap_or("not-loaded").to_string());
        }
    }

    /// Records a finished job and expands its dependencies.
    fn record(&mut self, done: Done) {
        let index = done.index;
        let (loaded, mut analysis, map) = match done.result {
            Ok(v) => v,
            Err(reason) => {
                self.resources[index].failure = Some(reason);
                return;
            }
        };
        self.bytes += loaded.bytes;
        if self.bytes > self.options.budgets.max_bytes {
            self.stop("total-byte-budget");
        }
        let has_fatal = analysis.gaps.iter().any(|g| g.reason == "parse-error");
        let resource = &mut self.resources[index];
        resource.url = loaded.final_url.clone();
        resource.status = if has_fatal {
            SourceStatus::Failed
        } else if analysis.gaps.is_empty() {
            SourceStatus::Completed
        } else {
            SourceStatus::Partial
        };
        resource.source_map = map;
        // HTML resources define the base/import map for their children;
        // other resources inherit from their referrer (set when enqueued).
        if let Some(m) = &analysis.import_map {
            resource.import_map = Some(m.clone());
        }
        let own_url = resource.url.clone();
        let html_base = analysis
            .base_url
            .clone()
            .or_else(|| resource.document_base.clone());
        let depth = resource.depth;
        let import_map = resource.import_map.clone();
        // Each finite candidate becomes an independent edge. This preserves
        // failures as well as successes instead of silently choosing one URL.
        analysis.dependencies = analysis
            .dependencies
            .into_iter()
            .flat_map(|dep| {
                if dep.specifiers.len() <= 1 {
                    return vec![dep];
                }
                dep.specifiers
                    .iter()
                    .map(|specifier| {
                        let mut candidate = dep.clone();
                        candidate.specifiers = vec![specifier.clone()];
                        candidate
                    })
                    .collect()
            })
            .collect();
        resource.loaded = Some(loaded);

        // Standalone JS entries use the caller-provided import map.
        let import_map = import_map.or_else(|| {
            self.options.import_map.as_ref().and_then(|m| {
                resolve::parse_import_map(m, self.options.base_url.as_deref().unwrap_or(&own_url))
                    .ok()
            })
        });

        for (dep_index, dep) in analysis.dependencies.iter().enumerate() {
            let edge = self.expand(
                index,
                dep_index,
                dep,
                &own_url,
                html_base.as_deref(),
                depth,
                &import_map,
            );
            self.resources[index].edges.push(edge);
        }
        self.resources[index].analysis = Some(analysis);
    }

    /// Resolves one dependency and enqueues the child when appropriate.
    #[allow(clippy::too_many_arguments)]
    fn expand(
        &mut self,
        from: usize,
        dep_index: usize,
        dep: &Dependency,
        own_url: &str,
        html_base: Option<&str>,
        depth: usize,
        import_map: &Option<ImportMap>,
    ) -> Edge {
        let kind = dep.kind.child_kind();
        let single = self.options.dependency_mode == DependencyMode::Single;

        // A child document has its own global object. Keep the reference,
        // but do not load or analyze its scripts in the parent scan.
        if dep.kind == DependencyKind::Iframe {
            return Edge {
                dependency: dep_index,
                to: None,
                status: DependencyStatus::NotFollowed,
                reason: Some("iframe-isolated-global".into()),
                url: dep.specifiers.first().and_then(|specifier| {
                    resolve::join(dep.base_url.as_deref().unwrap_or(own_url), specifier).ok()
                }),
            };
        }

        // Inline content: child URL is a fragment of the parent.
        if let Some(content) = &dep.content {
            let offset = dep.content_offset.unwrap_or(dep.span.start);
            let url = format!(
                "{}#inline@{}",
                own_url.split('#').next().unwrap_or(own_url),
                offset
            );
            if kind == Kind::Css {
                // CSS is discovered, never analyzed.
                return Edge {
                    dependency: dep_index,
                    to: None,
                    status: DependencyStatus::NotFollowed,
                    reason: Some("css-not-analyzed".into()),
                    url: None,
                };
            }
            if single {
                return Edge {
                    dependency: dep_index,
                    to: None,
                    status: DependencyStatus::NotFollowed,
                    reason: Some("single-mode".into()),
                    url: None,
                };
            }
            if depth + 1 > self.options.budgets.max_depth
                || self.resources.len() >= self.options.budgets.max_resources
            {
                let reason = if depth + 1 > self.options.budgets.max_depth {
                    "depth-budget"
                } else {
                    "resource-count-budget"
                };
                return Edge {
                    dependency: dep_index,
                    to: None,
                    status: DependencyStatus::Unresolved,
                    reason: Some(reason.into()),
                    url: None,
                };
            }
            let child = self.add(
                kind,
                url.clone(),
                dep.script_mode,
                depth + 1,
                Some(from),
                Some((from, offset)),
            );
            if kind != Kind::Html {
                self.resources[child].import_map = import_map.clone();
            }
            self.enqueue(
                child,
                Source::Content {
                    content: content.clone(),
                    resource_url: Some(url),
                },
                dep.base_url.clone().or(html_base.map(String::from)),
                depth + 1,
            );
            return Edge {
                dependency: dep_index,
                to: Some(child),
                status: DependencyStatus::Resolved,
                reason: None,
                url: None,
            };
        }

        // External: resolve the first specifier (candidates are rare; the
        // analyzer bounds them to a handful).
        // Static module imports use the importer URL. DOM loaders use the
        // document base carried in their dependency. Anonymous inputs can
        // fall back to the caller's explicit base URL.
        let base = dep.base_url.as_deref().unwrap_or_else(|| {
            if own_url.starts_with("memory:") {
                self.options.base_url.as_deref().unwrap_or(own_url)
            } else {
                own_url
            }
        });
        let mut resolved = None;
        let mut last_error = String::from("non-finite-dependency");
        for spec in &dep.specifiers {
            let r = if dep.kind.is_module_specifier() {
                resolve::resolve_module(spec, base, import_map.as_ref())
            } else {
                resolve::join(base, spec)
            };
            match r {
                Ok(url) => {
                    if let Err(e) = resolve::reject_credentials(&url) {
                        last_error = e;
                        continue;
                    }
                    resolved = Some(url);
                    break;
                }
                Err(e) => last_error = e,
            }
        }
        let Some(url) = resolved else {
            return Edge {
                dependency: dep_index,
                to: None,
                status: DependencyStatus::Unresolved,
                reason: Some(last_error),
                url: None,
            };
        };
        if self.exclude.is_match(&url) {
            return Edge {
                dependency: dep_index,
                to: None,
                status: DependencyStatus::NotFollowed,
                reason: Some("excluded".into()),
                url: Some(url),
            };
        }
        if kind == Kind::Css {
            return Edge {
                dependency: dep_index,
                to: None,
                status: DependencyStatus::NotFollowed,
                reason: Some("css-not-analyzed".into()),
                url: Some(url),
            };
        }
        if single {
            return Edge {
                dependency: dep_index,
                to: None,
                status: DependencyStatus::NotFollowed,
                reason: Some("single-mode".into()),
                url: Some(url),
            };
        }
        if dep.kind == DependencyKind::Worker {
            return Edge {
                dependency: dep_index,
                to: None,
                status: DependencyStatus::Unresolved,
                reason: Some("worker-environment-not-analyzed".into()),
                url: Some(url),
            };
        }
        if depth + 1 > self.options.budgets.max_depth {
            return Edge {
                dependency: dep_index,
                to: None,
                status: DependencyStatus::Unresolved,
                reason: Some("depth-budget".into()),
                url: Some(url),
            };
        }
        let mode = if kind == Kind::Js {
            Some(dep.script_mode.unwrap_or(self.options.script_mode))
        } else {
            None
        };
        let context = if self.resources[from].key_kind == Kind::Html {
            from + 1
        } else {
            self.resources[from].context
        };
        if !self.by_key.contains_key(&ResourceKey {
            kind,
            url: url.clone(),
            mode,
            context,
        }) && self.resources.len() >= self.options.budgets.max_resources
        {
            return Edge {
                dependency: dep_index,
                to: None,
                status: DependencyStatus::Unresolved,
                reason: Some("resource-count-budget".into()),
                url: Some(url),
            };
        }
        let child = self.add(kind, url.clone(), mode, depth + 1, Some(from), None);
        if kind != Kind::Html {
            self.resources[child].import_map = import_map.clone();
        }
        let source = self
            .options
            .resource_mappings
            .get(&url)
            .cloned()
            .unwrap_or(Source::Url { url: url.clone() });
        // Workers and iframes resolve against their own URL; everything else
        // against the document base of the referrer.
        let child_base = match dep.kind {
            DependencyKind::Worker | DependencyKind::Iframe => None,
            _ => html_base.map(String::from).or_else(|| dep.base_url.clone()),
        };
        self.enqueue(child, source, child_base, depth + 1);
        Edge {
            dependency: dep_index,
            to: Some(child),
            status: DependencyStatus::Resolved,
            reason: None,
            url: Some(url),
        }
    }

    /// Queues a resource for loading unless it already ran.
    fn enqueue(&mut self, index: usize, source: Source, base_url: Option<String>, depth: usize) {
        let r = &self.resources[index];
        if r.loaded.is_some() || r.failure.is_some() || self.in_flight.contains(&index) {
            return;
        }
        if self.queue.iter().any(|(i, _, _)| *i == index) {
            return;
        }
        let _ = depth;
        self.resources[index].document_base = base_url.clone();
        self.queue.push_back((index, source, base_url));
    }
}

#[cfg(test)]
#[path = "tests/scheduler.rs"]
mod tests;
