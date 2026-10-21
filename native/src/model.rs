use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;
pub fn hash(value: &Value) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}
pub fn content_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Source {
    File {
        path: String,
    },
    Url {
        url: String,
    },
    Content {
        content: String,
        #[serde(
            rename = "resourceUrl",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        resource_url: Option<String>,
    },
}
impl Source {
    pub fn url(&self) -> Result<String, String> {
        match self {
            Self::File { path } => {
                let p = PathBuf::from(path);
                Url::from_file_path(if p.is_absolute() {
                    p
                } else {
                    std::env::current_dir().map_err(|e| e.to_string())?.join(p)
                })
                .map(|u| u.to_string())
                .map_err(|_| "invalid-file-path".into())
            }
            Self::Url { url } => Ok(url.clone()),
            Self::Content {
                resource_url,
                content,
            } => Ok(resource_url
                .clone()
                .unwrap_or_else(|| format!("memory:{}", content_hash(content)))),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub kind: String,
    pub source: Source,
    pub resource_url: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub script_mode: String,
    pub execution_environment: String,
    pub scope_key: String,
    pub versions: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_map: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_map_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_path: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Budgets {
    pub concurrency: usize,
    pub analysis_concurrency: usize,
    pub timeout_ms: u64,
    pub resource_timeout_ms: u64,
    pub max_resources: usize,
    pub max_bytes: usize,
    pub max_resource_bytes: usize,
    pub max_buffered_bytes: usize,
    pub max_depth: usize,
    pub max_steps: usize,
    pub max_candidates: usize,
    pub max_summary_passes: usize,
    pub max_redirects: usize,
    pub retries: usize,
    pub max_records: usize,
}
impl Default for Budgets {
    fn default() -> Self {
        Self {
            concurrency: 4,
            analysis_concurrency: 2,
            timeout_ms: 120_000,
            resource_timeout_ms: 30_000,
            max_resources: 1000,
            max_bytes: 128 * 1024 * 1024,
            max_resource_bytes: 16 * 1024 * 1024,
            max_buffered_bytes: 64 * 1024 * 1024,
            max_depth: 32,
            max_steps: 200_000,
            max_candidates: 64,
            max_summary_passes: 8,
            max_redirects: 5,
            retries: 1,
            max_records: 500_000,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Work {
    pub kind: String,
    pub source: Source,
    pub resource_url: String,
    pub context: Context,
    pub depth: usize,
    pub incoming: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline: Option<Inline>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inline {
    pub parent_url: String,
    pub offset: usize,
    pub parent_content: String,
    #[serde(default)]
    pub approximate: bool,
}
#[derive(Clone)]
pub struct Loaded {
    pub content: String,
    pub final_url: String,
    pub bytes: usize,
}
#[derive(Clone, Deserialize)]
pub struct Request {
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub options: Value,
}
pub fn string(v: &Value, key: &str, default: &str) -> String {
    v[key].as_str().unwrap_or(default).into()
}
pub fn raw() -> Value {
    json!({"accesses":[],"dependencies":[],"findings":[],"unresolved":[],"exports":{},"diagnostics":[],"steps":0})
}
pub fn push(v: &mut Value, key: &str, item: Value) {
    v[key].as_array_mut().unwrap().push(item);
}
pub fn gap(v: &mut Value, start: usize, end: usize, reason: &str, cap: &str) {
    push(
        v,
        "unresolved",
        json!({"start":start,"end":end,"reason":reason,"affectedCapabilities":[cap]}),
    );
}
pub fn location(source: &str, start: usize, end: usize) -> Value {
    fn point(s: &str, at: usize) -> (usize, usize, usize) {
        let mut end = at.min(s.len());
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        let before = &s[..end];
        let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
        let column = before
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .encode_utf16()
            .count();
        (before.encode_utf16().count(), line, column)
    }
    let (a, l, c) = point(source, start);
    let (b, el, ec) = point(source, end);
    json!({"start":a,"end":b,"line":l,"column":c,"endLine":el,"endColumn":ec,"offsetEncoding":"utf16"})
}
pub fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

#[cfg(test)]
#[path = "tests/model.rs"]
mod tests;
