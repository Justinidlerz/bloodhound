use super::*;

impl Run {
    pub(super) fn drain(&mut self) -> Result<(), String> {
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
            let loading = self.timings.stage("load");
            let loaded = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .into_iter()
                    .map(|work| {
                        let options = &self.request.options;
                        let budgets = &self.budgets;
                        let flag = &self.flag;
                        let deadline = self.deadline;
                        let timer = self.timings.resource(&work);
                        scope.spawn(move || {
                            let _load = timer.task("load");
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
            drop(loading);
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
                let analysis = self.timings.stage("analysis");
                let analyzed = std::thread::scope(|scope| {
                    let handles: Vec<_> = chunk
                        .iter()
                        .map(|(work, loaded)| {
                            scope.spawn(move || {
                                let key = hash(&json!([
                                    content_hash(&loaded.content),
                                    work.kind,
                                    work.context
                                ]));
                                if let Some(raw) = this.cache.get(&key) {
                                    let timer = this.timings.resource(work);
                                    let _cache = timer.task("cacheHit");
                                    Ok(raw.clone())
                                } else {
                                    this.raw_analysis(work, loaded, json!({}))
                                }
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
                drop(analysis);
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
    pub(super) fn summarize(&mut self) -> Result<(), String> {
        if self.dependency_mode != "recursive" {
            return Ok(());
        }
        let _linking = self.timings.stage("moduleLinking");
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
                let analysis = self.timings.stage("analysis");
                let result = self.raw_analysis(&work, &loaded, imports);
                drop(analysis);
                let raw = match result {
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
