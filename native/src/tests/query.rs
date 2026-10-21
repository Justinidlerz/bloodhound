use super::*;
use crate::test_support::{content, js_report, scan};
fn index() -> Index {
    Index::new(scan(
        json!([
            content(
                "js",
                "https://example.test/a.js",
                "globalThis.shared=1;globalThis.shared;"
            ),
            content(
                "js",
                "https://example.test/b.js",
                "globalThis.shared='s';shared;"
            )
        ]),
        json!({"dependencyMode":"single"}),
    ))
    .unwrap()
}
fn target(index: &Index) -> String {
    index
        .invoke(
            "findTargets",
            &json!({"pathSegments":["shared"],"namespace":"global-object"}),
        )
        .unwrap()[0]["targetId"]
        .as_str()
        .unwrap()
        .into()
}
#[test]
fn target_queries_filter_namespace_and_scope() {
    let index = index();
    assert_eq!(
        index
            .invoke("findTargets", &json!({"pathSegments":["shared"]}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(index.invoke("findTargets",&json!({"pathSegments":["shared"],"namespace":"global-object","scopeKey":"test-scope"})).unwrap().as_array().unwrap().len(),1);
    assert_eq!(
        index
            .invoke(
                "findTargets",
                &json!({"pathSegments":["shared"],"scopeKey":"other"})
            )
            .unwrap(),
        json!([])
    );
}
#[test]
fn source_and_resource_queries_keep_access_owners() {
    let index = index();
    let id = target(&index);
    let sources = index.invoke("sources", &json!({"id":id})).unwrap();
    assert_eq!(sources.as_array().unwrap().len(), 3);
    for source in sources.as_array().unwrap() {
        assert_eq!(
            source["access"]["analysisId"],
            source["analysis"]["analysisId"]
        );
        assert_eq!(
            source["analysis"]["resourceId"],
            source["resource"]["resourceId"]
        );
        let by_analysis = index
            .invoke("accesses", &json!({"id":source["analysis"]["analysisId"]}))
            .unwrap();
        let by_resource = index
            .invoke(
                "resourceAccesses",
                &json!({"id":source["resource"]["resourceId"]}),
            )
            .unwrap();
        assert_eq!(by_analysis, by_resource);
    }
    for op in ["sources", "accesses", "resourceAccesses"] {
        assert_eq!(
            index.invoke(op, &json!({"id":"missing"})).unwrap(),
            json!([])
        );
    }
}
#[test]
fn conflicts_and_unbound_resolution_preserve_candidate_status() {
    let index = index();
    let conflicts = index.invoke("conflicts", &json!({})).unwrap();
    assert_eq!(conflicts.as_array().unwrap().len(), 1);
    assert_eq!(conflicts[0]["certainty"], "static-candidate");
    assert_eq!(conflicts[0]["accessIds"].as_array().unwrap().len(), 2);
    let unbound = index.invoke("resolveUnbound", &json!({})).unwrap();
    assert_eq!(unbound[0]["candidates"], json!([target(&index)]));
    assert_eq!(unbound[0]["certainty"], "static-candidate");
}
#[test]
fn entry_paths_paginate_and_bound_traversal() {
    let index = index();
    let id = target(&index);
    let all = index.invoke("entryPaths", &json!({"id":id})).unwrap();
    assert_eq!(all["paths"].as_array().unwrap().len(), 2);
    let first = index
        .invoke("entryPaths", &json!({"id":id,"limit":1}))
        .unwrap();
    assert_eq!(first["nextOffset"], 1);
    assert_eq!(first["truncated"], true);
    let second = index
        .invoke("entryPaths", &json!({"id":id,"offset":1,"limit":1}))
        .unwrap();
    assert_ne!(first["paths"][0], second["paths"][0]);
    for _ in 0..32 {
        assert_eq!(
            index
                .invoke("entryPaths", &json!({"id":id,"limit":1}))
                .unwrap(),
            first
        );
        assert_eq!(
            index
                .invoke("entryPaths", &json!({"id":id,"offset":1,"limit":1}))
                .unwrap(),
            second
        );
    }
    let bounded = index
        .invoke("entryPaths", &json!({"id":id,"maxVisits":0}))
        .unwrap();
    assert_eq!(bounded["paths"], json!([]));
    assert_eq!(bounded["truncated"], true);
    assert_eq!(
        index
            .invoke("entryPaths", &json!({"id":"missing"}))
            .unwrap()["truncated"],
        false
    );
}
#[test]
fn recursive_entry_paths_preserve_reference_order_without_cycles() {
    let report = scan(
        json!([content(
            "js",
            "https://example.test/a.js",
            "import './b.js'"
        )]),
        json!({"resourceMappings":{"https://example.test/b.js":{"type":"content","resourceUrl":"https://example.test/b.js","content":"import './a.js';globalThis.shared=1"},"https://example.test/a.js":{"type":"content","resourceUrl":"https://example.test/a.js","content":"import './b.js'"}}}),
    );
    let index = Index::new(report).unwrap();
    let paths = index
        .invoke("entryPaths", &json!({"id":target(&index)}))
        .unwrap();
    assert_eq!(paths["paths"].as_array().unwrap().len(), 1);
    assert_eq!(
        paths["paths"][0]["analysisIds"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        paths["paths"][0]["referenceIds"].as_array().unwrap().len(),
        1
    );
}
#[test]
fn adapters_and_index_artifacts_retain_report_identity() {
    let index = index();
    let artifact = index
        .invoke("indexArtifact", &json!({"reportId":"external"}))
        .unwrap();
    assert_eq!(artifact["reportId"], "external");
    assert!(artifact.get("configuration").is_none());
    assert_eq!(artifact["globalAccesses"], index.report["globalAccesses"]);
    for kind in ["build", "page"] {
        let report = index
            .invoke(
                "adaptReport",
                &json!({"type":kind,"metadata":{"release":"r1"}}),
            )
            .unwrap();
        assert_eq!(report["reportType"], kind);
        assert_eq!(report["scope"], json!({"release":"r1"}));
        assert_eq!(report["resourceResults"].as_array().unwrap().len(), 2);
        assert_eq!(report["index"]["reportId"], index.report["scanId"]);
    }
    assert_eq!(
        index
            .invoke("adaptReport", &json!({"type":"other"}))
            .unwrap_err(),
        "invalid-report-type"
    );
    assert_eq!(
        index.invoke("unknown", &json!({})).unwrap_err(),
        "unknown-index-operation"
    );
    assert!(Index::new(json!({})).is_err());
}
#[test]
fn runtime_candidates_apply_every_filter_without_claiming_execution() {
    let index = index();
    let source = &index
        .invoke("sources", &json!({"id":target(&index)}))
        .unwrap()[0];
    let args = json!({"resourceUrl":source["resource"]["finalUrl"],"contentHash":source["resource"]["contentHash"],"line":source["access"]["location"]["line"],"column":source["access"]["location"]["column"],"pathSegments":["shared"]});
    let candidates = index.invoke("runtimeCandidates", &args).unwrap();
    assert_eq!(candidates.as_array().unwrap().len(), 1);
    assert_eq!(candidates[0]["attribution"], "static_candidate");
    for key in [
        "resourceUrl",
        "contentHash",
        "line",
        "column",
        "pathSegments",
    ] {
        let mut bad = args.clone();
        bad[key] = match key {
            "line" | "column" => json!(99999),
            "pathSegments" => json!(["missing"]),
            _ => json!("missing"),
        };
        assert_eq!(
            index.invoke("runtimeCandidates", &bad).unwrap(),
            json!([]),
            "{key}"
        );
    }
}
#[test]
fn diffs_ignore_location_shifts_but_identify_type_changes_and_additions() {
    let before = js_report("globalThis.shared=1;");
    let index = Index::new(before).unwrap();
    let shifted = js_report("\n\nglobalThis.shared=1;");
    assert_eq!(
        index
            .invoke("diffReports", &json!({"after":shifted}))
            .unwrap(),
        json!({"added":[],"removed":[],"changed":[]})
    );
    let changed = js_report("globalThis.shared='x';");
    assert_eq!(
        index
            .invoke("diffReports", &json!({"after":changed}))
            .unwrap()["changed"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let added = js_report("globalThis.shared=1;globalThis.extra=true;");
    assert!(
        !index
            .invoke("diffReports", &json!({"after":added}))
            .unwrap()["added"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(index.invoke("diffReports", &json!({"after":{}})).is_err());
}
#[test]
fn ambiguous_diff_matches_become_additions_and_removals() {
    let report = js_report("globalThis.shared=1;globalThis.shared=1;");
    let result = diff(&report, &report);
    assert!(result["changed"].as_array().unwrap().is_empty());
    assert!(!result["added"].as_array().unwrap().is_empty());
    assert_eq!(result["added"], result["removed"]);
}
