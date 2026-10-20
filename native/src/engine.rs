use crate::{js, markup, model::*, resolve};
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
    time::Duration,
};
use url::Url;
const COLLECTIONS: [&str; 7] = [
    "resources",
    "analysisInstances",
    "references",
    "globalTargets",
    "globalAccesses",
    "findings",
    "unresolved",
];
const SCHEMA: &str = include_str!("../../schemas/report.schema.json");
static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
pub fn validate(report: &Value) -> Result<(), String> {
    let v = VALIDATOR.get_or_init(|| {
        jsonschema::validator_for(&serde_json::from_str::<Value>(SCHEMA).unwrap()).unwrap()
    });
    if let Err(e) = v.validate(report) {
        return Err(format!("invalid-report: {e}"));
    }
    let ids = |key: &str, id: &str| {
        report[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| text(&v[id]).to_string())
            .collect::<HashSet<_>>()
    };
    let resources = ids("resources", "resourceId");
    let analyses = ids("analysisInstances", "analysisId");
    let targets = ids("globalTargets", "targetId");
    let accesses = ids("globalAccesses", "accessId");
    for a in report["analysisInstances"].as_array().unwrap() {
        if !resources.contains(text(&a["resourceId"])) {
            return Err("dangling-resource".into());
        }
    }
    for r in report["references"].as_array().unwrap() {
        if !analyses.contains(text(&r["fromAnalysisId"]))
            || r["toAnalysisIds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| !analyses.contains(text(id)))
        {
            return Err("dangling-reference".into());
        }
    }
    for a in report["globalAccesses"].as_array().unwrap() {
        if !analyses.contains(text(&a["analysisId"]))
            || a["targetRefs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| !targets.contains(text(id)))
        {
            return Err("dangling-access".into());
        }
    }
    for f in report["findings"].as_array().unwrap() {
        if !analyses.contains(text(&f["analysisId"]))
            || f["accessRefs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| !accesses.contains(text(id)))
        {
            return Err("dangling-finding".into());
        }
    }
    Ok(())
}
fn date() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap()
}
fn authorize(value: &str, options: &Value) -> Result<Url, String> {
    let u = Url::parse(value).map_err(|_| "invalid-url")?;
    if !u.username().is_empty() || u.password().is_some() {
        return Err("url-credentials-prohibited".into());
    }
    let protocol = format!("{}:", u.scheme());
    if let Some(allowed) = options["access"]["protocols"].as_array() {
        if !allowed.iter().any(|v| v == &protocol) {
            return Err("protocol-not-allowed".into());
        }
    } else if !["file", "http", "https"].contains(&u.scheme()) {
        return Err("protocol-not-allowed".into());
    }
    if u.scheme() == "file" {
        if let Some(roots) = options["access"]["fileRoots"].as_array() {
            let path = fs::canonicalize(u.to_file_path().map_err(|_| "invalid-file-url")?)
                .map_err(|e| e.to_string())?;
            if !roots
                .iter()
                .any(|r| fs::canonicalize(text(r)).is_ok_and(|r| path.starts_with(r)))
            {
                return Err("file-outside-allowed-roots".into());
            }
        }
    } else if let Some(hosts) = options["access"]["hosts"].as_array() {
        let host = &u[url::Position::BeforeHost..url::Position::AfterPort];
        if !hosts.iter().any(|v| v.as_str() == Some(host)) {
            return Err("host-not-allowed".into());
        }
    }
    Ok(u)
}
fn stopped(flag: &AtomicBool, deadline: u64) -> Result<(), String> {
    if flag.load(Ordering::Relaxed) {
        Err("cancelled".into())
    } else if now() >= deadline {
        Err("total-time-budget".into())
    } else {
        Ok(())
    }
}
fn read_bounded(
    mut reader: impl Read,
    limit: usize,
    flag: &AtomicBool,
    deadline: u64,
) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    let mut buffer = [0u8; 16384];
    loop {
        stopped(flag, deadline)?;
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if result.len().saturating_add(n) > limit {
            return Err("resource-byte-budget".into());
        }
        result.extend_from_slice(&buffer[..n]);
    }
    Ok(result)
}
fn load(
    source: &Source,
    options: &Value,
    budgets: &Budgets,
    flag: &AtomicBool,
    deadline: u64,
) -> Result<Loaded, String> {
    stopped(flag, deadline)?;
    if let Source::Content {
        content,
        resource_url,
    } = source
    {
        if content.len() > budgets.max_resource_bytes {
            return Err("resource-byte-budget".into());
        }
        return Ok(Loaded {
            content: content.clone(),
            final_url: resource_url.clone(),
            bytes: content.len(),
        });
    }
    let initial = source.url()?;
    let parsed = authorize(&initial, options)?;
    if parsed.scheme() == "file" {
        let file = File::open(parsed.to_file_path().map_err(|_| "invalid-file-url")?)
            .map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > budgets.max_resource_bytes as u64 {
            return Err("resource-byte-budget".into());
        }
        let bytes = read_bounded(file, budgets.max_resource_bytes, flag, deadline)?;
        let count = bytes.len();
        return Ok(Loaded {
            content: String::from_utf8(bytes).map_err(|_| "invalid-utf8")?,
            final_url: initial,
            bytes: count,
        });
    }
    let mut builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(
            budgets
                .resource_timeout_ms
                .min(deadline.saturating_sub(now()))
                .max(1),
        ));
    if let Some(proxy) = options["proxy"].as_str() {
        builder =
            builder.proxy(reqwest::Proxy::all(proxy).map_err(|e| e.without_url().to_string())?);
    }
    let client = builder.build().map_err(|e| e.without_url().to_string())?;
    let mut last = "resource-load-failed".to_string();
    for _ in 0..=budgets.retries {
        let mut current = initial.clone();
        for redirect in 0..=budgets.max_redirects {
            stopped(flag, deadline)?;
            let u = authorize(&current, options)?;
            let mut req = client.get(u.clone());
            let host = &u[url::Position::BeforeHost..url::Position::AfterPort];
            if let Some(headers) = options["requestHeaders"][host].as_object() {
                for (k, v) in headers {
                    req = req.header(k, text(v));
                }
            }
            let response = match req.send() {
                Ok(r) => r,
                Err(e) => {
                    last = e.without_url().to_string();
                    break;
                }
            };
            if response.status().is_redirection() {
                if redirect == budgets.max_redirects {
                    return Err("redirect-budget".into());
                }
                current = resolve::join(
                    &current,
                    response
                        .headers()
                        .get("location")
                        .and_then(|h| h.to_str().ok())
                        .ok_or("redirect-without-location")?,
                )?;
                continue;
            }
            if !response.status().is_success() {
                last = format!("http-status-{}", response.status().as_u16());
                break;
            }
            if response
                .content_length()
                .is_some_and(|n| n > budgets.max_resource_bytes as u64)
            {
                return Err("resource-byte-budget".into());
            }
            let bytes = read_bounded(response, budgets.max_resource_bytes, flag, deadline)?;
            let count = bytes.len();
            return Ok(Loaded {
                content: String::from_utf8(bytes).map_err(|_| "invalid-utf8")?,
                final_url: current,
                bytes: count,
            });
        }
    }
    Err(last)
}
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
}
impl Run {
    fn emit(&mut self, kind: &str, value: &Value) -> Result<(), String> {
        if let Some(stream) = &mut self.stream {
            serde_json::to_writer(&mut *stream, &json!({"type":kind,"value":value}))
                .map_err(|e| e.to_string())?;
            stream.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn add(&mut self, key: &str, value: Value) -> Result<(), String> {
        if self.record_count >= self.budgets.max_records && key != "unresolved" {
            return Err("record-budget".into());
        }
        self.record_count += 1;
        push(&mut self.report, key, value.clone());
        self.emit(key, &value)
    }
    fn gap(
        &mut self,
        reason: &str,
        id: Option<&str>,
        reference: Option<&str>,
        detail: Option<&str>,
    ) -> Result<(), String> {
        let mut v = json!({"reason":reason,"affectedCapabilities":["resources","dependencies"]});
        if let Some(id) = id {
            v["analysisId"] = json!(id);
        }
        if let Some(id) = reference {
            v["referenceId"] = json!(id);
        }
        if let Some(detail) = detail {
            v["detail"] = json!(detail);
        }
        self.add("unresolved", v)
    }
    fn key(work: &Work) -> String {
        hash(&json!([work.resource_url, work.kind, work.context]))
    }
    fn connect(&mut self, incoming: &[String], id: &str) {
        for r in self.report["references"].as_array_mut().unwrap() {
            if incoming.iter().any(|x| x == text(&r["referenceId"]))
                && !r["toAnalysisIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|x| x == id)
            {
                r["toAnalysisIds"].as_array_mut().unwrap().push(json!(id));
            }
        }
    }
    fn enqueue(&mut self, work: Work) -> Result<(), String> {
        let key = Self::key(&work);
        if let Some(id) = self.seen.get(&key).cloned() {
            self.connect(&work.incoming, &id);
            return Ok(());
        }
        if let Some(pending) = self.queue.iter_mut().find(|p| Self::key(p) == key) {
            pending.incoming.extend(work.incoming);
            return Ok(());
        }
        if work.depth > self.budgets.max_depth {
            self.gap(
                "depth-budget",
                None,
                work.incoming.first().map(String::as_str),
                None,
            )?;
            return Ok(());
        }
        self.queue.push_back(work);
        Ok(())
    }
    fn raw_analysis(&self, work: &Work, loaded: &Loaded, imports: Value) -> Result<Value, String> {
        stopped(&self.flag, self.deadline)?;
        match work.kind.as_str() {
            "js" => {
                let mut options = serde_json::to_value(&work.context).unwrap();
                options["maxSteps"] = json!(self.budgets.max_steps);
                options["maxCandidates"] = json!(self.budgets.max_candidates);
                options["importedConstants"] = imports;
                options["resourceUrl"] = json!(if work.resource_url != work.source.url()? {
                    &work.resource_url
                } else {
                    &loaded.final_url
                });
                options["deadlineUnixMs"] =
                    json!(self.deadline.min(now() + self.budgets.resource_timeout_ms));
                options["runId"] = self.request.options["runId"].clone();
                let result = js::analyze(loaded.content.clone(), options.to_string())
                    .map_err(|e| e.to_string())?;
                serde_json::from_str(&result).map_err(|e| e.to_string())
            }
            "html" => Ok(markup::html(
                &loaded.content,
                if work.resource_url != work.source.url()? {
                    &work.resource_url
                } else {
                    &loaded.final_url
                },
                &work.context,
                self.budgets.max_steps,
            )),
            "css" => Ok(markup::css(&loaded.content, self.budgets.max_steps)),
            _ => Err("unsupported-resource-kind".into()),
        }
    }
    fn source_map(&self, loaded: &Loaded) -> Option<sourcemap::DecodedMap> {
        if self.request.options["sourceMaps"] != true {
            return None;
        }
        let marker = "sourceMappingURL=";
        let offset = loaded.content.rfind(marker)? + marker.len();
        let reference = loaded.content[offset..]
            .split(|c: char| c.is_whitespace() || c == '*')
            .next()?;
        let bytes = if let Some(data) = reference.strip_prefix("data:") {
            let (metadata, body) = data.split_once(',')?;
            if metadata.ends_with(";base64") {
                base64::engine::general_purpose::STANDARD
                    .decode(body)
                    .ok()?
            } else {
                percent_decode(body)?
            }
        } else {
            let url = resolve::join(&loaded.final_url, reference).ok()?;
            load(
                &Source::Url { url },
                &self.request.options,
                &self.budgets,
                &self.flag,
                self.deadline,
            )
            .ok()?
            .content
            .into_bytes()
        };
        if bytes.len() > self.budgets.max_resource_bytes {
            return None;
        }
        sourcemap::decode_slice(&bytes).ok()
    }
    fn process(
        &mut self,
        work: Work,
        loaded: Loaded,
        override_raw: Option<Value>,
    ) -> Result<(), String> {
        let final_url = if work.resource_url != work.source.url()?
            || matches!(work.source, Source::Content { .. })
        {
            work.resource_url.clone()
        } else {
            loaded.final_url.clone()
        };
        let resource_id = hash(&json!([
            final_url,
            content_hash(&loaded.content),
            work.kind
        ]));
        let context_hash = hash(&serde_json::to_value(&work.context).unwrap());
        let id = hash(&json!([resource_id, context_hash]));
        self.seen.insert(Self::key(&work), id.clone());
        self.connect(&work.incoming, &id);
        if work.depth == 0
            && !self.report["entryAnalysisIds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x == &id)
        {
            push(&mut self.report, "entryAnalysisIds", json!(id));
        }
        if self.report["analysisInstances"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["analysisId"] == id)
        {
            return Ok(());
        }
        let mut resource = json!({"resourceId":resource_id,"requestedUrl":resolve::sanitize_url(&work.resource_url),"finalUrl":resolve::sanitize_url(&final_url),"contentHash":content_hash(&loaded.content),"kind":work.kind,"bytes":loaded.bytes});
        if let Some(scm) = work.context.versions.get("scmId") {
            resource["scmId"] = json!(scm);
        }
        if let Some(inline) = &work.inline {
            resource["inline"] = json!({"parentUrl":inline.parent_url,"offset":inline.offset});
            if inline.approximate {
                resource["inline"]["approximate"] = json!(true);
            }
        }
        if !self.report["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["resourceId"] == resource_id)
        {
            self.add("resources", resource.clone())?;
        }
        let mut instance = json!({"analysisId":id,"resourceId":resource_id,"contextHash":context_hash,"context":work.context,"scanStatus":"completed","exports":{},"sourceMapStatus":"absent"});
        self.add("analysisInstances", instance.clone())?;
        let cache_key = hash(&json!([resource["contentHash"], work.kind, work.context]));
        let raw = if let Some(raw) = override_raw {
            raw
        } else if let Some(raw) = self.cache.get(&cache_key) {
            raw.clone()
        } else {
            match self.raw_analysis(&work, &loaded, json!({})) {
                Ok(raw) => raw,
                Err(e) => {
                    instance["scanStatus"] = json!("failed");
                    if let Some(existing) = self.report["analysisInstances"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|a| a["analysisId"] == id)
                    {
                        *existing = instance.clone();
                    }
                    self.emit("analysisInstances", &instance)?;
                    self.gap("analysis-failed", Some(&id), None, Some(&e))?;
                    return Ok(());
                }
            }
        };
        let size = raw.to_string().len();
        if self.cache_bytes + size < self.budgets.max_bytes / 4 {
            self.cache_bytes += size;
            self.cache.insert(cache_key, raw.clone());
        }
        if work.kind == "js" {
            self.snapshots
                .insert(id.clone(), (work.clone(), loaded.clone(), raw.clone()));
        }
        let map = self.source_map(&loaded);
        if map.is_some() {
            instance["sourceMapStatus"] = json!("mapped");
        } else if self.request.options["sourceMaps"] == true
            && loaded.content.contains("sourceMappingURL=")
        {
            instance["sourceMapStatus"] = json!("unavailable");
        }
        let locate = |span: &Value| {
            let start = span["start"].as_u64().unwrap_or(0) as usize;
            let end = span["end"].as_u64().unwrap_or(0) as usize;
            let mut l = location(&loaded.content, start, end);
            if let Some(map) = &map {
                if let Some(token) = map.lookup_token(
                    l["line"].as_u64().unwrap() as u32 - 1,
                    l["column"].as_u64().unwrap() as u32,
                ) {
                    if let Some(source) = token.get_source() {
                        let mut original = json!({"source":source,"line":token.get_src_line()+1,"column":token.get_src_col()});
                        if let Some(name) = token.get_name() {
                            original["name"] = json!(name);
                        }
                        l["original"] = original;
                    }
                }
            }
            if let Some(inline) = &work.inline {
                let p = location(
                    &inline.parent_content,
                    inline.offset + if inline.approximate { 0 } else { start },
                    inline.offset + if inline.approximate { 0 } else { end },
                );
                l["container"] = json!({"resourceUrl":inline.parent_url,"offset":p["start"],"line":p["line"],"column":p["column"]});
                if inline.approximate {
                    l["container"]["approximate"] = json!(true);
                }
            }
            l
        };
        instance["exports"] = raw["exports"].clone();
        for u in raw["unresolved"].as_array().unwrap() {
            let mut item = u.clone();
            item["analysisId"] = json!(id);
            item["location"] = locate(u);
            self.add("unresolved", item)?;
        }
        if !raw["unresolved"].as_array().unwrap().is_empty() {
            instance["scanStatus"] = json!(if raw["unresolved"]
                .as_array()
                .unwrap()
                .iter()
                .any(|u| ["parse-error", "css-parse-error", "analysis-failed"]
                    .contains(&text(&u["reason"])))
            {
                "failed"
            } else {
                "partial"
            });
        }
        for (access_index, a) in raw["accesses"].as_array().unwrap().iter().enumerate() {
            if access_index % 128 == 0 {
                stopped(&self.flag, self.deadline)?;
            }
            let mut refs = Vec::new();
            for t in a["targets"].as_array().unwrap() {
                let target_id = hash(&json!([
                    work.context.scope_key,
                    t["namespace"],
                    t["pathSegments"],
                    t["keyKind"],
                    if t["keyKind"] == "symbol-local" {
                        json!(id)
                    } else {
                        Value::Null
                    }
                ]));
                refs.push(target_id.clone());
                if !self.report["globalTargets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t["targetId"] == target_id)
                {
                    let mut target = t.clone();
                    target["scopeKey"] = json!(work.context.scope_key);
                    target["targetId"] = json!(target_id);
                    self.add("globalTargets", target)?;
                }
            }
            let loc = locate(a);
            let fingerprint = hash(&json!([
                final_url,
                work.context.scope_key,
                work.context.script_mode,
                work.context.execution_environment,
                a["targets"],
                a["operation"],
                a["valueType"],
                a["evidence"]
            ]));
            let access_id = hash(&json!([id, loc, a["operation"], refs]));
            if self.report["globalAccesses"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["accessId"] == access_id)
            {
                continue;
            }
            let access = json!({"accessId":access_id,"analysisId":id,"targetRefs":refs,"operation":a["operation"],"valueType":a["valueType"],"location":loc,"evidence":a["evidence"],"fingerprint":fingerprint});
            self.add("globalAccesses", access)?;
            if ["write", "declare", "define", "delete"].contains(&text(&a["operation"])) {
                let prototype = a["targets"].as_array().unwrap().iter().any(|t| {
                    t["pathSegments"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|s| s == "prototype")
                });
                self.add("findings",json!({"findingId":hash(&json!([access_id,"global-mutation"])),"ruleId":if prototype{"prototype-mutation"}else{"global-mutation"},"kind":"global","analysisId":id,"accessRefs":[access_id],"location":loc,"details":{"certainty":"static-candidate","operation":a["operation"]},"fingerprint":hash(&json!([fingerprint,"global-mutation"]))}))?;
            }
        }
        for f in raw["findings"].as_array().unwrap() {
            self.add("findings",json!({"findingId":hash(&json!([id,f])),"ruleId":f["ruleId"],"kind":f["kind"],"analysisId":id,"accessRefs":[],"location":locate(f),"details":f["details"],"fingerprint":hash(&json!([final_url,work.context.scope_key,f["ruleId"],f["details"]]))}))?;
        }
        // Add instance before dependency processing so every reference has an owner even on budget exhaustion.
        if let Some(existing) = self.report["analysisInstances"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|a| a["analysisId"] == id)
        {
            *existing = instance.clone();
        }
        self.emit("analysisInstances", &instance)?;
        for d in raw["dependencies"].as_array().unwrap() {
            let specs = if d.get("content").is_some() {
                vec![Value::Null]
            } else {
                let s = d["specifiers"].as_array().cloned().unwrap_or_default();
                if s.is_empty() { vec![Value::Null] } else { s }
            };
            for specifier in specs {
                let rid = hash(&json!([id, d, specifier]));
                let mut reference = json!({"referenceId":rid,"fromAnalysisId":id,"toAnalysisIds":[],"kind":d["kind"],"resolution":"unresolved","location":locate(d)});
                if let Some(s) = specifier.as_str() {
                    reference["specifier"] = json!(resolve::sanitize_url(s));
                }
                for key in ["attributes", "condition"] {
                    if d.get(key).is_some_and(|v| !v.is_null()) {
                        reference[key] = d[key].clone();
                    }
                }
                let mut ctx = work.context.clone();
                let base = d["baseUrl"]
                    .as_str()
                    .or(if work.inline.is_some() {
                        work.context.base_url.as_deref()
                    } else {
                        None
                    })
                    .unwrap_or(&final_url);
                ctx.base_url = Some(base.into());
                if let Some(m) = d.get("importMap") {
                    ctx.import_map = Some(m.clone());
                    ctx.import_map_base_url = Some(base.into());
                }
                if let Some(mode) = d["scriptMode"].as_str() {
                    ctx.script_mode = mode.into();
                }
                let child = if d.get("content").is_some() {
                    Ok(format!(
                        "{}#inline-{}",
                        final_url.split('#').next().unwrap_or(&final_url),
                        hash(&json!([final_url, d["kind"], d["start"]]))
                    ))
                } else if let Some(spec) = specifier.as_str() {
                    resolve::resolve(
                        spec,
                        base,
                        &ctx,
                        ["import", "export", "dynamic-import"].contains(&text(&d["kind"])),
                    )
                } else {
                    Err("non-finite-dependency".into())
                };
                match child {
                    Err(e) => {
                        reference["reason"] = json!(e);
                        self.gap("dependency-resolution", Some(&id), Some(&rid), Some(&e))?;
                        self.add("references", reference)?;
                    }
                    Ok(url) => {
                        if Url::parse(&url)
                            .is_ok_and(|u| !u.username().is_empty() || u.password().is_some())
                        {
                            reference["reason"] = json!("url-credentials-prohibited");
                            self.add("references", reference)?;
                            self.gap("url-credentials-prohibited", Some(&id), Some(&rid), None)?;
                            continue;
                        }
                        reference["resolvedUrl"] = json!(url);
                        reference["resolution"] =
                            json!(if d["specifiers"].as_array().is_some_and(|a| a.len() > 1) {
                                "candidate"
                            } else {
                                "resolved"
                            });
                        let kind = resolve::kind(d, &url);
                        if ["worker", "iframe"].contains(&text(&d["kind"])) {
                            ctx.scope_key = hash(&json!([
                                work.context.scope_key,
                                final_url,
                                d["start"],
                                d["kind"]
                            ]));
                            ctx.execution_environment = if d["kind"] == "worker" {
                                "worker"
                            } else {
                                "window"
                            }
                            .into();
                            ctx.import_map = None;
                            ctx.import_map_base_url = None;
                        }
                        if kind == "asset" || self.dependency_mode == "single" {
                            reference["resolution"] = json!("not-followed");
                            reference["reason"] = json!(if kind == "asset" {
                                "non-code-resource"
                            } else {
                                "single-resource-scope"
                            });
                            self.add("references", reference)?;
                        } else {
                            let source = if let Some(content) = d["content"].as_str() {
                                Source::Content {
                                    content: content.into(),
                                    resource_url: url.clone(),
                                }
                            } else if let Some(mapping) =
                                self.request.options["resourceMappings"].get(&url)
                            {
                                serde_json::from_value(mapping.clone())
                                    .map_err(|e| e.to_string())?
                            } else {
                                Source::Url { url: url.clone() }
                            };
                            self.add("references", reference)?;
                            self.enqueue(Work {
                                kind,
                                source,
                                resource_url: url,
                                context: ctx,
                                depth: work.depth + 1,
                                incoming: vec![rid],
                                inline: if d.get("content").is_some() {
                                    Some(Inline {
                                        parent_url: final_url.clone(),
                                        offset: d["start"].as_u64().unwrap_or(0) as usize,
                                        parent_content: loaded.content.clone(),
                                        approximate: d["sourceLocationApproximate"] == true,
                                    })
                                } else {
                                    None
                                },
                            })?;
                        }
                    }
                }
            }
        }
        if let Some(stream) = &mut self.stream {
            stream.flush().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn checkpoint(&self) -> Result<(), String> {
        if let Some(path) = self.request.options["checkpointPath"].as_str() {
            let mut options = json!({});
            for key in ["access", "resourceMappings", "sourceMaps"] {
                if let Some(value) = self.request.options.get(key) {
                    options[key] = value.clone();
                }
            }
            let checkpoint = json!({"checkpointVersion":1,"configHash":self.config_hash,"entries":self.request.entries,"pending":self.queue,"report":self.report,"options":options});
            write_json(path, &checkpoint)?;
        }
        Ok(())
    }
    fn drain(&mut self) -> Result<(), String> {
        while !self.queue.is_empty()
            && stopped(&self.flag, self.deadline).is_ok()
            && self.bytes <= self.budgets.max_bytes
        {
            if self.count >= self.budgets.max_resources {
                self.gap("resource-count-budget", None, None, None)?;
                break;
            }
            let n = self
                .budgets
                .concurrency
                .min((self.budgets.max_buffered_bytes / self.budgets.max_resource_bytes).max(1))
                .min(self.budgets.max_resources - self.count)
                .min(self.queue.len());
            let batch: Vec<_> = self.queue.drain(..n).collect();
            self.count += n;
            let loaded = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .into_iter()
                    .map(|work| {
                        let options = &self.request.options;
                        let budgets = &self.budgets;
                        let flag = &self.flag;
                        let deadline = self.deadline;
                        scope.spawn(move || {
                            let result = load(&work.source, options, budgets, flag, deadline);
                            (work, result)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().map_err(|_| "fetch-thread-failed".to_string()))
                    .collect::<Result<Vec<_>, _>>()
            })?;
            let mut ready = Vec::new();
            for (work, result) in loaded {
                match result {
                    Err(e) => self.gap(
                        "resource-load-failed",
                        None,
                        work.incoming.first().map(String::as_str),
                        Some(&format!(
                            "{}: {e}",
                            resolve::sanitize_url(&work.resource_url)
                        )),
                    )?,
                    Ok(loaded) => {
                        self.bytes += loaded.bytes;
                        if self.bytes > self.budgets.max_bytes {
                            self.gap(
                                "total-byte-budget",
                                None,
                                work.incoming.first().map(String::as_str),
                                None,
                            )?;
                            self.queue.push_back(work);
                        } else {
                            ready.push((work, loaded));
                        }
                    }
                }
            }
            for chunk in ready.chunks(self.budgets.analysis_concurrency) {
                let this: &Run = self;
                let analyzed = std::thread::scope(|scope| {
                    let handles: Vec<_> =
                        chunk
                            .iter()
                            .map(|(work, loaded)| {
                                scope.spawn(move || {
                                    let key = hash(&json!([
                                        content_hash(&loaded.content),
                                        work.kind,
                                        work.context
                                    ]));
                                    this.cache.get(&key).cloned().map(Ok).unwrap_or_else(|| {
                                        this.raw_analysis(work, loaded, json!({}))
                                    })
                                })
                            })
                            .collect();
                    handles
                        .into_iter()
                        .map(|h| {
                            h.join()
                                .unwrap_or_else(|_| Err("analysis-thread-failed".into()))
                        })
                        .collect::<Vec<_>>()
                });
                for ((work, loaded), result) in chunk.iter().zip(analyzed) {
                    let raw = match result {
                        Ok(raw) => raw,
                        Err(error) => {
                            let mut result = raw();
                            push(
                                &mut result,
                                "unresolved",
                                json!({"start":0,"end":loaded.content.len(),"reason":"analysis-failed","detail":error,"affectedCapabilities":["resources"]}),
                            );
                            result
                        }
                    };
                    self.process(work.clone(), loaded.clone(), Some(raw))?;
                }
            }
            self.checkpoint()?;
        }
        Ok(())
    }
    fn summarize(&mut self) -> Result<(), String> {
        if self.dependency_mode != "recursive" {
            return Ok(());
        }
        let mut changed = false;
        for _ in 0..self.budgets.max_summary_passes {
            if stopped(&self.flag, self.deadline).is_err() {
                break;
            }
            changed = false;
            let ids: Vec<_> = self.snapshots.keys().cloned().collect();
            for id in ids {
                let mut imports = json!({});
                for r in self.report["references"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| {
                        r["fromAnalysisId"] == id
                            && ["import", "export"].contains(&text(&r["kind"]))
                    })
                {
                    if let Some(spec) = r["specifier"].as_str() {
                        for to in r["toAnalysisIds"].as_array().unwrap() {
                            if let Some(a) = self.report["analysisInstances"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .find(|a| &a["analysisId"] == to)
                            {
                                for (name, value) in a["exports"].as_object().into_iter().flatten()
                                {
                                    imports[format!("{spec}#{name}")] = value.clone();
                                }
                            }
                        }
                    }
                }
                if imports.as_object().unwrap().is_empty() {
                    continue;
                }
                let (work, loaded, old) = self.snapshots[&id].clone();
                let raw = match self.raw_analysis(&work, &loaded, imports) {
                    Ok(r) => r,
                    Err(e) => {
                        self.gap("summary-analysis-failed", Some(&id), None, Some(&e))?;
                        continue;
                    }
                };
                if raw == old {
                    continue;
                }
                changed = true;
                self.emit("resetAnalysis", &json!({"analysisId":id}))?;
                for key in [
                    "analysisInstances",
                    "globalAccesses",
                    "findings",
                    "unresolved",
                ] {
                    self.report[key]
                        .as_array_mut()
                        .unwrap()
                        .retain(|v| v["analysisId"] != id);
                }
                self.report["references"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|v| v["fromAnalysisId"] != id);
                self.process(work, loaded, Some(raw))?;
            }
            self.drain()?;
            if !changed {
                break;
            }
        }
        if changed {
            self.gap("module-summary-pass-budget", None, None, None)?;
        }
        Ok(())
    }
}
fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}
pub fn write_json(path: &str, value: &Value) -> Result<(), String> {
    let target = Path::new(path);
    let temp = target.with_extension(format!("{}.tmp", now()));
    let mut file = File::create(&temp).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, value).map_err(|e| e.to_string())?;
    file.write_all(b"\n").map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    fs::rename(&temp, target).map_err(|e| e.to_string())
}
pub fn scan(input: &str, flag: Arc<AtomicBool>) -> Result<String, String> {
    let mut request: Request = serde_json::from_str(input).map_err(|e| e.to_string())?;
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
    };
    if let Err(e) = run.drain().and_then(|_| run.summarize()) {
        run.gap(&e, None, None, None)?;
    }
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
    validate(&run.report)?;
    run.checkpoint()?;
    let mut summary = run.report.clone();
    for key in COLLECTIONS {
        summary.as_object_mut().unwrap().remove(key);
    }
    run.emit("summary", &summary)?;
    if let Some(stream) = &mut run.stream {
        stream.flush().map_err(|e| e.to_string())?;
    }
    if let Some(path) = run.request.options["reportPath"].as_str() {
        write_json(path, &run.report)?;
    }
    Ok(run.report.to_string())
}
