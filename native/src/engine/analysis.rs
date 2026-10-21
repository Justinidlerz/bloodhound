use super::*;

impl Run {
    pub(super) fn raw_analysis(
        &self,
        work: &Work,
        loaded: &Loaded,
        imports: Value,
    ) -> Result<Value, String> {
        let timer = self.timings.resource(work);
        let _analysis = timer.task("analysis");
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
                let result = if self.request.options["timings"] == true {
                    js::analyze_timed(loaded.content.clone(), options.to_string(), &timer)
                } else {
                    js::analyze(loaded.content.clone(), options.to_string())
                }
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
    pub(super) fn source_map(&self, loaded: &Loaded) -> Option<sourcemap::DecodedMap> {
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
    pub(super) fn process(
        &mut self,
        work: Work,
        loaded: Loaded,
        override_raw: Option<Value>,
    ) -> Result<(), String> {
        let timer = self.timings.resource(&work);
        let _records = timer.stage("records");
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
            let analysis = self.timings.stage("analysis");
            let result = self.raw_analysis(&work, &loaded, json!({}));
            drop(analysis);
            match result {
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
        let map = {
            let _source_maps = timer.stage("sourceMaps");
            self.source_map(&loaded)
        };
        if map.is_some() {
            instance["sourceMapStatus"] = json!("mapped");
        } else if self.request.options["sourceMaps"] == true
            && loaded.content.contains("sourceMappingURL=")
        {
            instance["sourceMapStatus"] = json!("unavailable");
        }
        let locate = |span: &Value| {
            let _locations = timer.stage("locations");
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
                let known_target = {
                    let _deduplication = timer.stage("deduplication");
                    self.report["globalTargets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|t| t["targetId"] == target_id)
                };
                if !known_target {
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
            let known_access = {
                let _deduplication = timer.stage("deduplication");
                self.report["globalAccesses"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["accessId"] == access_id)
            };
            if known_access {
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
        let _dependencies = timer.stage("dependencies");
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
                ctx.base_url = Url::parse(base)
                    .ok()
                    .filter(|url| !url.cannot_be_a_base())
                    .map(|url| url.to_string());
                if let Some(m) = d.get("importMap") {
                    ctx.import_map = Some(m.clone());
                    ctx.import_map_base_url = ctx.base_url.clone();
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
                                    resource_url: Some(url.clone()),
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
}
pub(super) fn percent_decode(s: &str) -> Option<Vec<u8>> {
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
