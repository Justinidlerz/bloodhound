use super::*;
use serde_json::json;

#[test]
fn browser_resolution_handles_scopes_prefixes_and_blocking() {
    let map = parse_import_map(
        &json!({
            "imports":{"lib":"./lib.js", "pkg/":"./vendor/", "blocked":null},
            "scopes":{"./nested/":{"lib":"./nested-lib.js"}}
        }),
        "https://example.test/",
    )
    .unwrap();
    assert_eq!(
        resolve_module("lib", "https://example.test/nested/main.js", Some(&map)).unwrap(),
        "https://example.test/nested-lib.js"
    );
    assert_eq!(
        resolve_module("pkg/a.js", "https://example.test/main.js", Some(&map)).unwrap(),
        "https://example.test/vendor/a.js"
    );
    assert_eq!(
        resolve_module("blocked", "https://example.test/", Some(&map)).unwrap_err(),
        "import-map-blocked"
    );
    assert_eq!(
        resolve_module("pkg/../escape.js", "https://example.test/", Some(&map)).unwrap_err(),
        "import-map-backtracking"
    );
    assert_eq!(
        resolve_module("missing", "https://example.test/", None).unwrap_err(),
        "unmapped-bare-specifier"
    );
    assert!(join("memory:abc", "./a.js").is_err());
}

#[test]
fn merging_preserves_prior_mappings_and_credentials_are_redacted() {
    let a = parse_import_map(&json!({"imports":{"x":"./a.js"}}), "https://example.test/").unwrap();
    let b = parse_import_map(
        &json!({"imports":{"x":"./b.js", "y":"./y.js"}}),
        "https://example.test/",
    )
    .unwrap();
    let merged = merge_import_maps(&a, &b);
    assert_eq!(
        merged.imports["x"].as_deref(),
        Some("https://example.test/a.js")
    );
    assert!(merged.imports.contains_key("y"));
    assert_eq!(
        sanitize("https://user:secret@example.test/a"),
        "https://example.test/a"
    );
    assert!(reject_credentials("https://user:secret@example.test/a").is_err());
}
