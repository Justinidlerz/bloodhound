use super::*;

#[test]
fn hashes_are_stable_and_sensitive_to_content() {
    assert_eq!(
        content_hash("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(hash(&json!({"a":1,"b":2})), hash(&json!({"b":2,"a":1})));
    assert_ne!(hash(&json!([1, 2])), hash(&json!([2, 1])));
    assert_ne!(content_hash("a"), content_hash("a\n"));
}
#[test]
fn locations_use_utf16_and_clamp_to_character_boundaries() {
    let source = "a😀\r\nzé";
    assert_eq!(
        location(source, 1, 5),
        json!({"start":1,"end":3,"line":1,"column":1,"endLine":1,"endColumn":3,"offsetEncoding":"utf16"})
    );
    assert_eq!(location(source, 7, source.len())["column"], 0);
    assert_eq!(location(source, 7, source.len())["line"], 2);
    assert_eq!(location(source, 2, usize::MAX)["start"], 1);
    assert_eq!(location(source, 2, usize::MAX)["end"], 7);
}
#[test]
fn source_urls_preserve_logical_addresses_and_escape_file_paths() {
    for value in [
        json!({"type":"url","url":"https://example.test/a?q=1"}),
        json!({"type":"content","resourceUrl":"https://example.test/a?q=1","content":""}),
    ] {
        let source: Source = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(source.url().unwrap(), "https://example.test/a?q=1");
        assert_eq!(serde_json::to_value(source).unwrap(), value);
    }
    let source = Source::File {
        path: "fixture with spaces.js".into(),
    };
    let url = Url::parse(&source.url().unwrap()).unwrap();
    assert_eq!(
        url.to_file_path().unwrap(),
        std::env::current_dir()
            .unwrap()
            .join("fixture with spaces.js")
    );
    assert!(url.as_str().contains("%20"));
}
#[test]
fn raw_results_record_gaps_without_inventing_accesses() {
    let mut out = raw();
    gap(&mut out, 1, 3, "unknown-input", "globals");
    assert_eq!(
        out["unresolved"][0],
        json!({"start":1,"end":3,"reason":"unknown-input","affectedCapabilities":["globals"]})
    );
    assert_eq!(out["accesses"], json!([]));
    assert_eq!(string(&json!({"x":3}), "x", "fallback"), "fallback");
    assert_eq!(text(&Value::Null), "");
}
#[test]
fn budgets_serialize_with_public_camel_case_names() {
    let budgets = serde_json::to_value(Budgets::default()).unwrap();
    assert!(budgets["maxResourceBytes"].as_u64().unwrap() > 0);
    assert!(
        budgets["maxBufferedBytes"].as_u64().unwrap()
            >= budgets["maxResourceBytes"].as_u64().unwrap()
    );
    let decoded: Budgets = serde_json::from_value(budgets.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), budgets);
}
