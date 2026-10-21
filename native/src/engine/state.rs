use super::*;

impl Run {
    pub(super) fn emit(&mut self, kind: &str, value: &Value) -> Result<(), String> {
        if let Some(stream) = &mut self.stream {
            serde_json::to_writer(&mut *stream, &json!({"type":kind,"value":value}))
                .map_err(|e| e.to_string())?;
            stream.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub(super) fn add(&mut self, key: &str, value: Value) -> Result<(), String> {
        if self.record_count >= self.budgets.max_records && key != "unresolved" {
            return Err("record-budget".into());
        }
        self.record_count += 1;
        push(&mut self.report, key, value.clone());
        self.emit(key, &value)
    }
    pub(super) fn gap(
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
    pub(super) fn key(work: &Work) -> String {
        hash(&json!([work.resource_url, work.kind, work.context]))
    }
    pub(super) fn connect(&mut self, incoming: &[String], id: &str) {
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
    pub(super) fn enqueue(&mut self, work: Work) -> Result<(), String> {
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
    pub(super) fn checkpoint(&self) -> Result<(), String> {
        if let Some(path) = self.request.options["checkpointPath"].as_str() {
            let _checkpoint = self.timings.stage("checkpoint");
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
}
