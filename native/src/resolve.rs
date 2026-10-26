//! URL resolution following browser semantics.
//!
//! Resources are addressed by URL. Relative specifiers resolve against the
//! importer (or the document base for inline code). Module specifiers also
//! consult the import map: bare specifiers without a mapping are an error,
//! exactly as the browser would refuse to load them. There is no
//! `node_modules` lookup and no extension inference.

use crate::model::ImportMap;
use url::Url;

/// `true` for specifiers the browser treats as URLs rather than bare names.
pub fn is_url_like(specifier: &str) -> bool {
    specifier.starts_with('/')
        || specifier.starts_with("./")
        || specifier.starts_with("../")
        || Url::parse(specifier).is_ok()
}

/// Joins `specifier` against `base`. Absolute specifiers are returned as-is.
pub fn join(base: &str, specifier: &str) -> Result<String, String> {
    if let Ok(url) = Url::parse(specifier) {
        return Ok(url.to_string());
    }
    let base = Url::parse(base).map_err(|_| "invalid-base-url".to_string())?;
    if base.cannot_be_a_base() {
        return Err("missing-base-url".into());
    }
    base.join(specifier)
        .map(|u| u.to_string())
        .map_err(|_| "invalid-specifier".into())
}

/// Parses and normalizes an import map: every key and address is resolved
/// against `base`, and prefix mappings must end with `/` on both sides.
pub fn parse_import_map(raw: &serde_json::Value, base: &str) -> Result<ImportMap, String> {
    if !raw.is_object() {
        return Err("invalid-import-map".into());
    }
    fn entries(
        raw: Option<&serde_json::Value>,
        base: &str,
    ) -> Result<std::collections::BTreeMap<String, Option<String>>, String> {
        let mut out = std::collections::BTreeMap::new();
        let Some(raw) = raw else { return Ok(out) };
        let map = raw.as_object().ok_or("invalid-import-map")?;
        for (key, value) in map {
            if key.is_empty() {
                return Err("invalid-import-map-key".into());
            }
            let key = if is_url_like(key) {
                join(base, key)?
            } else {
                key.clone()
            };
            let address = match value {
                serde_json::Value::Null => None,
                serde_json::Value::String(s) if is_url_like(s) => {
                    let address = join(base, s)?;
                    if key.ends_with('/') && !address.ends_with('/') {
                        return Err("invalid-import-map-prefix".into());
                    }
                    Some(address)
                }
                _ => return Err("invalid-import-map-address".into()),
            };
            out.insert(key, address);
        }
        Ok(out)
    }
    let imports = entries(raw.get("imports"), base)?;
    let mut scopes = std::collections::BTreeMap::new();
    if let Some(raw_scopes) = raw.get("scopes") {
        let map = raw_scopes.as_object().ok_or("invalid-import-map-scopes")?;
        for (scope, value) in map {
            scopes.insert(join(base, scope)?, entries(Some(value), base)?);
        }
    }
    Ok(ImportMap { imports, scopes })
}

/// Merges a later import map into an earlier one. Earlier entries win, which
/// matches how browsers treat multiple import maps on one page.
pub fn merge_import_maps(first: &ImportMap, second: &ImportMap) -> ImportMap {
    let mut merged = second.clone();
    for (k, v) in &first.imports {
        merged.imports.insert(k.clone(), v.clone());
    }
    for (scope, entries) in &first.scopes {
        let target = merged.scopes.entry(scope.clone()).or_default();
        for (k, v) in entries {
            target.insert(k.clone(), v.clone());
        }
    }
    merged
}

fn lookup(
    specifier: &str,
    map: &std::collections::BTreeMap<String, Option<String>>,
) -> Result<Option<String>, String> {
    let key = map
        .keys()
        .filter(|k| {
            k.as_str() == specifier || (k.ends_with('/') && specifier.starts_with(k.as_str()))
        })
        .max_by_key(|k| k.len());
    let Some(key) = key else { return Ok(None) };
    let address = map[key].as_ref().ok_or("import-map-blocked")?;
    if key == specifier {
        return Ok(Some(address.clone()));
    }
    let resolved = join(address, &specifier[key.len()..])?;
    if !resolved.starts_with(address) {
        return Err("import-map-backtracking".into());
    }
    Ok(Some(resolved))
}

/// Resolves a module specifier using the import map, falling back to URL
/// joining for URL-like specifiers.
pub fn resolve_module(
    specifier: &str,
    importer: &str,
    map: Option<&ImportMap>,
) -> Result<String, String> {
    let normalized = if is_url_like(specifier) {
        join(importer, specifier)?
    } else {
        specifier.to_string()
    };
    if let Some(map) = map {
        let mut scopes: Vec<_> = map
            .scopes
            .keys()
            .filter(|k| {
                k.as_str() == importer || (k.ends_with('/') && importer.starts_with(k.as_str()))
            })
            .collect();
        scopes.sort_by_key(|k| std::cmp::Reverse(k.len()));
        for scope in scopes {
            if let Some(url) = lookup(&normalized, &map.scopes[scope])? {
                return Ok(url);
            }
        }
        if let Some(url) = lookup(&normalized, &map.imports)? {
            return Ok(url);
        }
    }
    if is_url_like(specifier) {
        Ok(normalized)
    } else {
        Err("unmapped-bare-specifier".into())
    }
}

/// Removes credentials from a URL before it appears in output.
pub fn sanitize(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut u) => {
            if !u.username().is_empty() || u.password().is_some() {
                let _ = u.set_username("");
                let _ = u.set_password(None);
            }
            u.to_string()
        }
        Err(_) => url.to_string(),
    }
}

/// Rejects URLs embedding credentials. Never fetch with them.
pub fn reject_credentials(url: &str) -> Result<(), String> {
    if let Ok(u) = Url::parse(url) {
        if !u.username().is_empty() || u.password().is_some() {
            return Err("url-credentials-prohibited".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/resolve.rs"]
mod tests;
