use crate::{engine, model::Context};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

pub fn context() -> Context {
    Context {
        script_mode: "module".into(),
        execution_environment: "window".into(),
        scope_key: "test-scope".into(),
        versions: BTreeMap::new(),
        base_url: None,
        import_map: None,
        import_map_base_url: None,
        public_path: None,
    }
}
pub fn content(kind: &str, url: &str, code: &str) -> Value {
    json!({"kind":kind,"source":{"type":"content","resourceUrl":url,"content":code}})
}
pub fn options(extra: Value) -> Value {
    let mut options = json!({"reportFormat":"full","scriptMode":"module","executionEnvironment":"window","scopeKey":"test-scope","runId":""});
    for (k, v) in extra.as_object().unwrap() {
        options[k] = v.clone();
    }
    options
}
pub fn request(entries: Value, extra: Value) -> String {
    json!({"entries":entries,"options":options(extra)}).to_string()
}
pub fn scan(entries: Value, extra: Value) -> Value {
    let result = engine::scan(&request(entries, extra), Arc::new(AtomicBool::new(false))).unwrap();
    let report = serde_json::from_str(&result).unwrap();
    engine::validate(&report).unwrap();
    report
}
pub fn js_report(code: &str) -> Value {
    scan(
        json!([content("js", "https://example.test/app.js", code)]),
        json!({"dependencyMode":"single"}),
    )
}
pub fn has_reason(result: &Value, reason: &str) -> bool {
    result["unresolved"]
        .as_array()
        .unwrap()
        .iter()
        .any(|u| u["reason"] == reason)
}
pub fn rules(result: &Value) -> Vec<&str> {
    result["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["ruleId"].as_str())
        .collect()
}
static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
pub struct TempDir(pub PathBuf);
impl TempDir {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "bloodhound-test-{}-{}-{}",
            std::process::id(),
            crate::model::now(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    pub fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
