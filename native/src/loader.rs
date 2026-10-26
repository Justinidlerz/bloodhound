//! Resource loading with access control and byte budgets.
//!
//! The loader is the only module that touches the filesystem or the network.
//! It is stateless apart from a shared HTTP client, so the scheduler can call
//! it from many worker threads at once.

use crate::model::{Access, Options, Source};
use crate::resolve;
use crate::scheduler::Cancellation;
use std::fs::{self, File};
use std::io::Read;
use std::time::Duration;
use url::Url;

/// A loaded resource.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub content: String,
    /// URL after redirects. Equal to the requested URL for files and content.
    pub final_url: String,
    pub bytes: usize,
}

/// Shared loader state. Construct once per scan.
pub struct Loader {
    client: Option<reqwest::blocking::Client>,
    options: Options,
}

impl Loader {
    pub fn new(options: &Options) -> Result<Self, String> {
        let mut builder = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(
                options.budgets.resource_timeout_ms.max(1),
            ))
            .user_agent(concat!("bloodhound/", env!("CARGO_PKG_VERSION")));
        if let Some(proxy) = &options.proxy {
            builder =
                builder.proxy(reqwest::Proxy::all(proxy).map_err(|e| e.without_url().to_string())?);
        }
        let client = Some(builder.build().map_err(|e| e.without_url().to_string())?);
        Ok(Self {
            client,
            options: options.clone(),
        })
    }

    /// Loads a source, honoring access rules and byte limits.
    pub fn load(&self, source: &Source, cancel: &Cancellation) -> Result<Loaded, String> {
        cancel.check()?;
        let budgets = &self.options.budgets;
        match source {
            Source::Content { content, .. } => {
                if content.len() > budgets.max_resource_bytes {
                    return Err("resource-byte-budget".into());
                }
                Ok(Loaded {
                    content: content.clone(),
                    final_url: source.url()?,
                    bytes: content.len(),
                })
            }
            Source::File { .. } | Source::Url { .. } => {
                let url = source.url()?;
                let parsed = authorize(&url, &self.options.access)?;
                if parsed.scheme() == "file" {
                    self.load_file(&parsed, &url, cancel)
                } else {
                    self.load_http(&url, cancel)
                }
            }
        }
    }

    fn load_file(&self, parsed: &Url, url: &str, cancel: &Cancellation) -> Result<Loaded, String> {
        let path = parsed
            .to_file_path()
            .map_err(|_| "invalid-file-url".to_string())?;
        let file = File::open(&path).map_err(|e| format!("file-open-failed: {e}"))?;
        let limit = self.options.budgets.max_resource_bytes;
        if file.metadata().map_err(|e| e.to_string())?.len() > limit as u64 {
            return Err("resource-byte-budget".into());
        }
        let bytes = read_bounded(file, limit, cancel)?;
        let content = String::from_utf8(bytes).map_err(|_| "invalid-utf8".to_string())?;
        Ok(Loaded {
            bytes: content.len(),
            content,
            final_url: url.to_string(),
        })
    }

    fn load_http(&self, url: &str, cancel: &Cancellation) -> Result<Loaded, String> {
        let client = self.client.as_ref().ok_or("http-client-unavailable")?;
        let budgets = &self.options.budgets;
        let mut last_error = "resource-load-failed".to_string();
        for _attempt in 0..=budgets.retries {
            let mut current = url.to_string();
            for redirect in 0..=budgets.max_redirects {
                cancel.check()?;
                let parsed = authorize(&current, &self.options.access)?;
                let mut request = client.get(parsed.clone());
                let host = &parsed[url::Position::BeforeHost..url::Position::AfterPort];
                if let Some(headers) = self.options.request_headers.get(host) {
                    for (name, value) in headers {
                        request = request.header(name, value);
                    }
                }
                let response = match request.send() {
                    Ok(r) => r,
                    Err(e) => {
                        last_error = e.without_url().to_string();
                        break; // retry
                    }
                };
                if response.status().is_redirection() {
                    if redirect == budgets.max_redirects {
                        return Err("redirect-budget".into());
                    }
                    let location = response
                        .headers()
                        .get("location")
                        .and_then(|h| h.to_str().ok())
                        .ok_or("redirect-without-location")?;
                    current = resolve::join(&current, location)?;
                    continue;
                }
                if !response.status().is_success() {
                    last_error = format!("http-status-{}", response.status().as_u16());
                    break;
                }
                if response
                    .content_length()
                    .is_some_and(|n| n > budgets.max_resource_bytes as u64)
                {
                    return Err("resource-byte-budget".into());
                }
                let bytes = read_bounded(response, budgets.max_resource_bytes, cancel)?;
                let content = String::from_utf8(bytes).map_err(|_| "invalid-utf8".to_string())?;
                return Ok(Loaded {
                    bytes: content.len(),
                    content,
                    final_url: current,
                });
            }
        }
        Err(last_error)
    }
}

/// Validates a URL against the access policy and returns it parsed.
pub fn authorize(url: &str, access: &Access) -> Result<Url, String> {
    let parsed = Url::parse(url).map_err(|_| "invalid-url".to_string())?;
    resolve::reject_credentials(url)?;
    let scheme = format!("{}:", parsed.scheme());
    match &access.protocols {
        Some(allowed) if !allowed.iter().any(|p| p == &scheme) => {
            return Err("protocol-not-allowed".into());
        }
        None if !["file", "http", "https"].contains(&parsed.scheme()) => {
            return Err("protocol-not-allowed".into());
        }
        _ => {}
    }
    if parsed.scheme() == "file" {
        if let Some(roots) = &access.file_roots {
            let path = parsed
                .to_file_path()
                .map_err(|_| "invalid-file-url".to_string())?;
            let canonical =
                fs::canonicalize(&path).map_err(|e| format!("file-open-failed: {e}"))?;
            let inside = roots
                .iter()
                .any(|root| fs::canonicalize(root).is_ok_and(|root| canonical.starts_with(root)));
            if !inside {
                return Err("file-outside-allowed-roots".into());
            }
        }
    } else if let Some(hosts) = &access.hosts {
        let host = &parsed[url::Position::BeforeHost..url::Position::AfterPort];
        if !hosts.iter().any(|h| h == host) {
            return Err("host-not-allowed".into());
        }
    }
    Ok(parsed)
}

/// Reads at most `limit` bytes, checking for cancellation between chunks.
pub fn read_bounded(
    mut reader: impl Read,
    limit: usize,
    cancel: &Cancellation,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        cancel.check()?;
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if out.len() + n > limit {
            return Err("resource-byte-budget".into());
        }
        out.extend_from_slice(&buffer[..n]);
    }
    Ok(out)
}

/// Decodes the source map referenced by a `sourceMappingURL` comment, if
/// any. Inline data URLs are decoded directly; external maps are loaded
/// through the same loader (honoring `resource_mappings`).
pub fn source_map(
    loader: &Loader,
    content: &str,
    final_url: &str,
    cancel: &Cancellation,
) -> Option<sourcemap::DecodedMap> {
    use base64::Engine as _;
    // Only accept directives from parser-recognized comments. A string such
    // as `const x = 'sourceMappingURL=...'` must never trigger a map load.
    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, content, oxc_span::SourceType::mjs()).parse();
    let reference = parsed.program.comments.iter().rev().find_map(|comment| {
        let text = content.get(comment.span.start as usize..comment.span.end as usize)?;
        let text = text
            .strip_prefix("//")
            .or_else(|| text.strip_prefix("/*"))?;
        let directive = text
            .trim_start()
            .strip_prefix('#')
            .or_else(|| text.trim_start().strip_prefix('@'))?
            .trim_start();
        directive
            .strip_prefix("sourceMappingURL=")?
            .split(|c: char| c.is_whitespace() || c == '*')
            .next()
    })?;
    let bytes = if let Some(data) = reference.strip_prefix("data:") {
        let (meta, body) = data.split_once(',')?;
        if body.len() > loader.options.budgets.max_resource_bytes.saturating_mul(3) {
            return None;
        }
        if meta.ends_with(";base64") {
            base64::engine::general_purpose::STANDARD
                .decode(body)
                .ok()?
        } else {
            percent_decode(body)?
        }
    } else {
        let url = resolve::join(final_url, reference).ok()?;
        let source = loader
            .options
            .resource_mappings
            .get(&url)
            .cloned()
            .unwrap_or(Source::Url { url });
        loader.load(&source, cancel).ok()?.content.into_bytes()
    };
    if bytes.len() > loader.options.budgets.max_resource_bytes {
        return None;
    }
    sourcemap::decode_slice(&bytes).ok()
}

fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
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

#[cfg(test)]
#[path = "tests/loader.rs"]
mod tests;
