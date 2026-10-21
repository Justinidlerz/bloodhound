use super::*;

pub(super) fn authorize(value: &str, options: &Value) -> Result<Url, String> {
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
pub(super) fn stopped(flag: &AtomicBool, deadline: u64) -> Result<(), String> {
    if flag.load(Ordering::Relaxed) {
        Err("cancelled".into())
    } else if now() >= deadline {
        Err("total-time-budget".into())
    } else {
        Ok(())
    }
}
pub(super) fn read_bounded(
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
pub(super) fn load(
    source: &Source,
    options: &Value,
    budgets: &Budgets,
    flag: &AtomicBool,
    deadline: u64,
) -> Result<Loaded, String> {
    stopped(flag, deadline)?;
    if let Source::Content { content, .. } = source {
        if content.len() > budgets.max_resource_bytes {
            return Err("resource-byte-budget".into());
        }
        return Ok(Loaded {
            content: content.clone(),
            final_url: source.url()?,
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
