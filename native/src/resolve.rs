use crate::model::{Context, text};
use serde_json::{Map, Value, json};
use url::Url;
fn url_like(s: &str) -> bool {
    s.starts_with('/') || s.starts_with("./") || s.starts_with("../") || Url::parse(s).is_ok()
}
pub fn join(base: &str, s: &str) -> Result<String, String> {
    if let Ok(url) = Url::parse(s) {
        return Ok(url.to_string());
    }
    let base = Url::parse(base).map_err(|_| "invalid-resource-url")?;
    if base.cannot_be_a_base() {
        return Err("missing-base-url".into());
    }
    base.join(s)
        .map(|u| u.to_string())
        .map_err(|_| "invalid-resource-url".into())
}
pub fn normalize(map: &Value, base: &str) -> Result<Value, String> {
    fn normalize_entries(map: Option<&Value>, base: &str) -> Result<Value, String> {
        let mut result = Map::new();
        if let Some(map) = map {
            let map = map.as_object().ok_or("invalid-import-map")?;
            for (k, v) in map {
                let key = if url_like(k) {
                    join(base, k)?
                } else {
                    k.clone()
                };
                if v.is_null() {
                    result.insert(key, Value::Null);
                } else if let Some(v) = v.as_str() {
                    if !url_like(v) {
                        return Err("invalid-import-map-address".into());
                    }
                    let value = join(base, v)?;
                    if key.ends_with('/') && !value.ends_with('/') {
                        return Err("invalid-import-map-prefix".into());
                    }
                    result.insert(key, Value::String(value));
                } else {
                    return Err("invalid-import-map-address".into());
                }
            }
        }
        Ok(Value::Object(result))
    }
    let mut scopes = Map::new();
    if let Some(v) = map.get("scopes") {
        for (k, v) in v.as_object().ok_or("invalid-import-map-scopes")? {
            scopes.insert(join(base, k)?, normalize_entries(Some(v), base)?);
        }
    }
    Ok(json!({"imports":normalize_entries(map.get("imports"),base)?,"scopes":scopes}))
}
fn mapping(specifier: &str, map: &Value) -> Result<Option<String>, String> {
    let Some(map) = map.as_object() else {
        return Ok(None);
    };
    let key = map
        .keys()
        .filter(|k| {
            k.as_str() == specifier || (k.ends_with('/') && specifier.starts_with(k.as_str()))
        })
        .max_by_key(|k| k.len());
    let Some(key) = key else { return Ok(None) };
    let target = map[key].as_str().ok_or("import-map-blocked")?;
    if key == specifier {
        return Ok(Some(target.into()));
    }
    let result = join(target, &specifier[key.len()..])?;
    if !result.starts_with(target) {
        return Err("import-map-prefix-backtracking".into());
    }
    Ok(Some(result))
}
pub fn resolve(
    specifier: &str,
    importer: &str,
    context: &Context,
    module: bool,
) -> Result<String, String> {
    let normalized = if url_like(specifier) {
        join(importer, specifier)?
    } else {
        specifier.into()
    };
    if module {
        let map = normalize(
            context.import_map.as_ref().unwrap_or(&json!({})),
            context
                .import_map_base_url
                .as_deref()
                .or(context.base_url.as_deref())
                .unwrap_or(importer),
        )?;
        let mut scopes: Vec<_> = map["scopes"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| {
                k.as_str() == importer || (k.ends_with('/') && importer.starts_with(k.as_str()))
            })
            .collect();
        scopes.sort_by_key(|k| std::cmp::Reverse(k.len()));
        for key in scopes {
            if let Some(value) = mapping(&normalized, &map["scopes"][key])? {
                return Ok(value);
            }
        }
        if let Some(value) = mapping(&normalized, &map["imports"])? {
            return Ok(value);
        }
        if !url_like(specifier) {
            return Err("unmapped-bare-specifier".into());
        }
    }
    join(importer, specifier)
}
pub fn merge_maps(prior: &Value, next: &Value) -> Value {
    let mut map = next.clone();
    for (k, v) in prior["imports"].as_object().into_iter().flatten() {
        map["imports"][k] = v.clone();
    }
    for (scope, entries) in prior["scopes"].as_object().into_iter().flatten() {
        if map["scopes"].get(scope).is_none() {
            map["scopes"][scope] = json!({});
        }
        for (k, v) in entries.as_object().into_iter().flatten() {
            map["scopes"][scope][k] = v.clone();
        }
    }
    map
}
pub fn sanitize_url(value: &str) -> String {
    if let Ok(mut u) = Url::parse(value) {
        if !u.username().is_empty() || u.password().is_some() {
            let _ = u.set_username("");
            let _ = u.set_password(None);
        }
        u.to_string()
    } else {
        value.to_string()
    }
}
pub fn kind(dependency: &Value, url: &str) -> String {
    match text(&dependency["kind"]) {
        "iframe" => "html",
        "style" | "css-import" => "css",
        "css-url" => "asset",
        "preload" if Url::parse(url).is_ok_and(|u| u.path().ends_with(".css")) => "css",
        _ => "js",
    }
    .into()
}

#[cfg(test)]
#[path = "tests/resolve.rs"]
mod tests;
