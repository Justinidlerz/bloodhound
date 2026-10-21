use super::*;
use crate::{
    model::text,
    test_support::{content, scan},
};
use serde_json::{Value, json};
use std::collections::HashSet;

#[test]
fn compact_preserves_mutations_findings_edges_and_coverage_without_internal_hashes() {
    let report = scan(
        json!([content(
            "html",
            "https://example.test/page.html",
            "<script>window.shared = 1; window.shared; Object.defineProperty(window, 'defined', {value: 2}); delete window.removed;</script><style>body{--theme:red}</style><script src='./missing.js'></script>"
        )]),
        json!({"access":{"protocols":["file"]}}),
    );
    let manifest = compact(&report);
    let writes: Vec<_> = report["globalAccesses"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["operation"] != "read")
        .collect();
    assert_eq!(manifest["globals"].as_array().unwrap().len(), writes.len());
    for (actual, expected) in manifest["globals"].as_array().unwrap().iter().zip(writes) {
        assert_eq!(actual["operation"], expected["operation"]);
        assert_eq!(actual["location"]["line"], expected["location"]["line"]);
        assert_eq!(actual["location"]["column"], expected["location"]["column"]);
        assert!(!actual["targets"].as_array().unwrap().is_empty());
    }
    assert!(manifest["summary"]["omittedReadAccesses"].as_u64().unwrap() > 0);
    assert_eq!(manifest["scanStatus"], report["scanStatus"]);
    assert_eq!(manifest["runStatus"], report["runStatus"]);
    assert_eq!(
        manifest["dependencies"].as_array().unwrap().len(),
        report["references"].as_array().unwrap().len()
    );
    assert_eq!(
        manifest["findings"].as_array().unwrap().len(),
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["kind"] != "global")
            .count()
    );
    let count: u64 = manifest["unresolved"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["count"].as_u64().unwrap())
        .sum();
    assert_eq!(count, report["unresolved"].as_array().unwrap().len() as u64);
    let ids: HashSet<_> = manifest["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| text(&s["id"]))
        .collect();
    for d in manifest["dependencies"].as_array().unwrap() {
        assert!(ids.contains(text(&d["from"])));
        for id in d["to"].as_array().into_iter().flatten() {
            assert!(ids.contains(text(id)));
        }
    }
    let serialized = manifest.to_string();
    for key in [
        "scopeKey",
        "contentHash",
        "contextHash",
        "fingerprint",
        "accessId",
        "analysisId",
        "#inline-",
    ] {
        assert!(!serialized.contains(key), "{key}");
    }
}

#[test]
fn gap_grouping_keeps_counts_and_explicitly_labels_a_sample_position() {
    let mut report = crate::test_support::js_report("globalThis.shared = 1");
    report["unresolved"] = json!([
        {"analysisId":report["analysisInstances"][0]["analysisId"],"reason":"dynamic-property-key","affectedCapabilities":["globals"],"location":{"line":1,"column":3,"endLine":1,"endColumn":4}},
        {"analysisId":report["analysisInstances"][0]["analysisId"],"reason":"dynamic-property-key","affectedCapabilities":["globals"],"location":{"line":2,"column":8,"endLine":2,"endColumn":9}},
        {"reason":"cancelled","affectedCapabilities":["all"]}
    ]);
    report["runStatus"] = json!("cancelled");
    report["scanStatus"] = json!("partial");
    let result = compact(&report);
    assert_eq!(result["unresolved"].as_array().unwrap().len(), 2);
    assert_eq!(result["unresolved"][0]["count"], 2);
    assert_eq!(result["unresolved"][0]["exampleLocation"]["column"], 3);
    assert!(result["unresolved"][0].get("location").is_none());
    assert_eq!(result["summary"]["unresolved"], 3);
    assert_eq!(result["runStatus"], "cancelled");
    assert_eq!(result["scanStatus"], "partial");
}

#[test]
fn anonymous_text_has_no_fake_base_and_keeps_inline_positions_and_scopes() {
    let report = scan(
        json!([{"kind":"html","source":{"type":"content","content":"<script>window.x=1</script><iframe srcdoc='<script>window.x=2</script>'></iframe><script src='./relative.js'></script>"}}]),
        json!({}),
    );
    assert!(
        report["references"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == "missing-base-url")
    );
    assert!(
        report["analysisInstances"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["context"].get("baseUrl").is_none())
    );
    let result = compact(&report);
    assert!(!result.to_string().contains("memory:"));
    assert!(!result.to_string().contains("file:"));
    assert_eq!(result["globals"].as_array().unwrap().len(), 2);
    let scopes: HashSet<Value> = result["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["scope"].clone())
        .collect();
    assert_eq!(scopes.len(), 2);
    assert!(result["globals"][0]["location"].get("container").is_some());
}

#[test]
fn inline_parents_follow_edges_across_repeated_iframes_and_real_url_fragments_are_retained() {
    let report = scan(
        json!([content(
            "html",
            "https://example.test/root.html",
            "<iframe src='./child.html'></iframe><iframe src='./child.html'></iframe><script src='./external.js#inline-real-fragment'></script>"
        )]),
        json!({"resourceMappings":{
            "https://example.test/child.html":{"type":"content","content":"<script>window.child=1</script>"},
            "https://example.test/external.js#inline-real-fragment":{"type":"content","content":"window.external=1"}
        }}),
    );
    let result = compact(&report);
    assert!(
        result["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["url"] == "https://example.test/external.js#inline-real-fragment")
    );
    let sources = result["sources"].as_array().unwrap();
    let children: Vec<_> = sources
        .iter()
        .filter(|s| s.get("inline").is_some())
        .collect();
    assert_eq!(children.len(), 2);
    assert_ne!(
        children[0]["inline"]["parent"],
        children[1]["inline"]["parent"]
    );
    for child in children {
        let parent = sources
            .iter()
            .find(|s| s["id"] == child["inline"]["parent"])
            .unwrap();
        assert_eq!(child["scope"], parent["scope"]);
    }
}

#[test]
fn pending_inline_resources_keep_gap_evidence_without_leaking_virtual_hashes() {
    let report = scan(
        json!([content(
            "html",
            "https://example.test/page.html",
            "<script>window.x=1</script>"
        )]),
        json!({"budgets":{"maxDepth":0}}),
    );
    assert_eq!(report["resources"].as_array().unwrap().len(), 1);
    assert!(text(&report["references"][0]["resolvedUrl"]).contains("#inline-"));
    let result = compact(&report);
    assert_eq!(result["scanStatus"], "partial");
    assert_eq!(result["dependencies"][0]["status"], "unresolved");
    assert!(result["dependencies"][0].get("reason").is_some());
    assert!(!result.to_string().contains("#inline-"));
}
