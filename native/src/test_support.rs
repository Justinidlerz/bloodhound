//! Shared helpers for unit tests.

use crate::model::*;
use crate::report;
use crate::scheduler::{self, Cancellation, Graph};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Builds a content entry.
pub fn content(kind: Kind, url: &str, text: &str) -> Entry {
    Entry {
        kind,
        source: Source::Content {
            content: text.into(),
            resource_url: Some(url.into()),
        },
        resource_url: None,
        script_mode: None,
    }
}

/// Default options with the network disabled so tests never reach out.
pub fn options() -> Options {
    Options {
        script_mode: ScriptMode::Classic,
        access: Access {
            protocols: Some(vec![]),
            hosts: None,
            file_roots: None,
        },
        ..Options::default()
    }
}

/// Runs the scheduler on entries and returns the graph.
pub fn graph(entries: Vec<Entry>, options: Options) -> Graph {
    let request = Request { entries, options };
    scheduler::run(&request, Arc::new(AtomicBool::new(false))).expect("scan runs")
}

/// Runs a full scan and returns the manifest.
pub fn manifest(entries: Vec<Entry>, options: Options) -> Manifest {
    let mode = options.dependency_mode;
    report::build(graph(entries, options), mode)
}

/// Scans one JS snippet in classic mode and returns its manifest.
pub fn js(code: &str) -> Manifest {
    manifest(
        vec![content(Kind::Js, "https://example.test/app.js", code)],
        options(),
    )
}

/// Scans one JS snippet in module mode.
pub fn mjs(code: &str) -> Manifest {
    let mut entry = content(Kind::Js, "https://example.test/app.mjs", code);
    entry.script_mode = Some(ScriptMode::Module);
    manifest(vec![entry], options())
}

/// Runs the JS analyzer directly, without the scheduler.
pub fn analyze_js(code: &str, mode: ScriptMode) -> Analysis {
    crate::js::analyze(
        code,
        &crate::js::JsOptions {
            script_mode: mode,
            resource_url: "https://example.test/app.js".into(),
            base_url: Some("https://example.test/".into()),
            max_steps: 100_000,
            max_candidates: 16,
        },
        &Cancellation::never(),
    )
}

/// Rules of all findings in a manifest, in source order.
pub fn rules(m: &Manifest) -> Vec<Rule> {
    m.findings.iter().map(|f| f.rule).collect()
}

/// Targets of all findings in a manifest.
pub fn targets(m: &Manifest) -> Vec<&str> {
    m.findings.iter().map(|f| f.target.as_str()).collect()
}

pub fn has_gap(m: &Manifest, reason: &str) -> bool {
    m.gaps.iter().any(|g| g.reason == reason)
}

/// Serializes a request the way the Node bridge would.
pub fn request_json(entries: Vec<Entry>, options: Options) -> String {
    json!({ "entries": entries, "options": options }).to_string()
}

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// A temporary directory removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "bloodhound-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    pub fn url(&self, name: &str) -> String {
        url::Url::from_file_path(self.0.join(name))
            .unwrap()
            .to_string()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A tiny HTTP server for loader tests. Serves fixed routes on 127.0.0.1.
pub struct Server {
    pub base: String,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// `routes` maps a path (`/a.js`) to `(status, body)`. A status of 301
    /// treats `body` as the redirect target.
    pub fn new(routes: Vec<(&str, u16, &str)>) -> Self {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Vec<(String, u16, String)> = routes
            .into_iter()
            .map(|(p, s, b)| (p.into(), s, b.into()))
            .collect();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let handle = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buf = [0u8; 4096];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let text = String::from_utf8_lossy(&buf[..n]);
                        let path = text
                            .lines()
                            .next()
                            .and_then(|l| l.split_whitespace().nth(1))
                            .unwrap_or("/")
                            .to_string();
                        let (status, body) = routes
                            .iter()
                            .find(|(p, _, _)| *p == path)
                            .map(|(_, s, b)| (*s, b.clone()))
                            .unwrap_or((404, String::new()));
                        let response = if status == 301 {
                            format!(
                                "HTTP/1.1 301 Moved\r\nLocation: {body}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            )
                        } else {
                            format!(
                                "HTTP/1.1 {status} OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                        };
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                }
            }
        });
        Self {
            base: format!("http://127.0.0.1:{port}"),
            stop,
            handle: Some(handle),
        }
    }

    pub fn host(&self) -> String {
        self.base.trim_start_matches("http://").to_string()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
