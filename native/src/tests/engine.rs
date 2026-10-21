use super::analysis::percent_decode;
use super::*;
use crate::test_support::{self as support, TempDir, content, has_reason, js_report, request};
fn scan(entries: Value, options: Value) -> Value {
    support::scan(entries, options)
}
fn one(code: &str, extra: Value) -> Value {
    scan(
        json!([content("js", "https://example.test/app.js", code)]),
        extra,
    )
}
fn error(entries: Value, options: Value) -> String {
    super::scan(&request(entries, options), Arc::new(AtomicBool::new(false))).unwrap_err()
}
#[test]
fn invalid_requests_are_rejected_before_work_starts() {
    assert!(super::scan("{", Arc::new(AtomicBool::new(false))).is_err());
    assert_eq!(error(json!([]), json!({})), "at-least-one-entry-required");
    for (extra, expected) in [
        (json!({"scriptMode":"bad"}), "invalid-execution-context"),
        (
            json!({"executionEnvironment":"node"}),
            "invalid-execution-context",
        ),
        (json!({"dependencyMode":"all"}), "invalid-dependency-mode"),
        (json!({"budgets":{"unknown":1}}), "unknown-budget-unknown"),
        (
            json!({"budgets":{"concurrency":0}}),
            "invalid-budget-concurrency",
        ),
    ] {
        assert_eq!(
            error(
                json!([content("js", "https://example.test/a.js", "")]),
                extra
            ),
            expected
        );
    }
    assert_eq!(
        error(
            json!([content("asset", "https://example.test/a", "")]),
            json!({})
        ),
        "unsupported-entry-kind"
    );
    assert_eq!(
        error(
            json!([content("js", "https://user:secret@example.test/a.js", "")]),
            json!({})
        ),
        "url-credentials-prohibited"
    );
}
#[test]
fn complete_reports_have_consistent_ids_versions_and_locations() {
    let report = one(
        "// 😀\nglobalThis.shared=1;",
        json!({"versions":{"scanner":"caller","scmId":"revision"}}),
    );
    assert_eq!(report["scanStatus"], "completed");
    assert_eq!(report["coverage"]["analyzed"], 1);
    assert_eq!(report["scannerVersion"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        report["configuration"]["context"]["versions"]["scanner"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(report["resources"][0]["scmId"], "revision");
    let write = report["globalAccesses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["operation"] == "write")
        .unwrap();
    assert_eq!(write["location"]["line"], 2);
    assert_eq!(write["location"]["start"], 6);
    assert_eq!(
        report["findings"][0]["details"]["certainty"],
        "static-candidate"
    );
}
#[test]
fn single_mode_records_edges_without_fetching_dependencies() {
    let report = one(
        "import './unavailable.js';globalThis.x=1",
        json!({"dependencyMode":"single"}),
    );
    assert_eq!(report["resources"].as_array().unwrap().len(), 1);
    assert_eq!(report["references"][0]["resolution"], "not-followed");
    assert_eq!(report["references"][0]["reason"], "single-resource-scope");
    assert_eq!(report["scanStatus"], "completed");
}
#[test]
fn recursive_graph_deduplicates_cycles_and_resolves_module_summaries() {
    let report = one(
        "import {key} from './dep.js'; globalThis[key]=1;",
        json!({"resourceMappings":{"https://example.test/dep.js":{"type":"content","resourceUrl":"https://example.test/dep.js","content":"import './app.js';export const key='shared';"},"https://example.test/app.js":{"type":"content","resourceUrl":"https://example.test/app.js","content":"import {key} from './dep.js'; globalThis[key]=1;"}}}),
    );
    assert_eq!(report["resources"].as_array().unwrap().len(), 2);
    assert_eq!(
        report["references"].as_array().unwrap().len(),
        report["analysisInstances"].as_array().unwrap().len()
    );
    assert!(
        report["references"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| !r["toAnalysisIds"].as_array().unwrap().is_empty())
    );
    assert!(
        report["globalTargets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["pathSegments"] == json!(["shared"]))
    );
    assert_eq!(report["scanStatus"], "completed");
}
#[test]
fn resource_identity_keeps_equal_content_at_distinct_urls() {
    let report = scan(
        json!([
            content("js", "https://example.test/a.js", "globalThis.x=1"),
            content("js", "https://example.test/b.js", "globalThis.x=1"),
            content("js", "https://example.test/a.js", "globalThis.x=1")
        ]),
        json!({}),
    );
    assert_eq!(report["resources"].as_array().unwrap().len(), 2);
    assert_eq!(report["analysisInstances"].as_array().unwrap().len(), 2);
    assert_eq!(report["entryAnalysisIds"].as_array().unwrap().len(), 2);
    assert_eq!(
        report["resources"][0]["contentHash"],
        report["resources"][1]["contentHash"]
    );
}
#[test]
fn html_discovers_import_maps_and_isolates_iframe_scopes() {
    let html = r#"<base href="https://example.test/assets/"><script type="importmap">{"imports":{"config":"./config.js"}}</script><script type="module">import 'config';globalThis.shared=1</script><iframe srcdoc="&lt;script&gt;globalThis.shared=2&lt;/script&gt;"></iframe>"#;
    let report = scan(
        json!([content("html", "https://example.test/page.html", html)]),
        json!({"resourceMappings":{"https://example.test/assets/config.js":{"type":"content","resourceUrl":"https://example.test/assets/config.js","content":"export const value=1;"}}}),
    );
    let targets: Vec<_> = report["globalTargets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["pathSegments"] == json!(["shared"]))
        .collect();
    assert_eq!(targets.len(), 2);
    assert_ne!(targets[0]["scopeKey"], targets[1]["scopeKey"]);
    assert!(
        report["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["finalUrl"] == "https://example.test/assets/config.js")
    );
    assert!(
        report["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|resource| resource["inline"]["approximate"] == true)
    );
}
#[test]
fn css_assets_are_not_scanned_as_code() {
    let report = scan(
        json!([content(
            "css",
            "https://example.test/style.css",
            "body{background:url('./image.png')}"
        )]),
        json!({}),
    );
    assert_eq!(report["references"][0]["resolution"], "not-followed");
    assert_eq!(report["references"][0]["reason"], "non-code-resource");
    assert_eq!(report["coverage"]["analyzed"], 1);
}
#[test]
fn parse_failures_do_not_hide_successful_independent_entries() {
    let report = scan(
        json!([
            content("js", "https://example.test/bad.js", "const = ;"),
            content("js", "https://example.test/good.js", "globalThis.x=1")
        ]),
        json!({}),
    );
    assert_eq!(report["scanStatus"], "partial");
    assert_eq!(report["coverage"]["failed"], 1);
    assert_eq!(report["coverage"]["analyzed"], 1);
    assert_eq!(one("const = ;", json!({}))["scanStatus"], "failed");
}
#[test]
fn depth_count_and_byte_budgets_leave_explicit_gaps() {
    let mappings = json!({"https://example.test/dep.js":{"type":"content","resourceUrl":"https://example.test/dep.js","content":"globalThis.x=1"}});
    for (budgets, reason) in [
        (json!({"maxDepth":0}), "depth-budget"),
        (json!({"maxResources":1}), "resource-count-budget"),
        (json!({"maxBytes":1}), "total-byte-budget"),
        (json!({"maxResourceBytes":1}), "resource-load-failed"),
    ] {
        let report = one(
            "import './dep.js';",
            json!({"budgets":budgets,"resourceMappings":mappings}),
        );
        assert!(has_reason(&report, reason), "{reason}: {report}");
        assert_ne!(report["scanStatus"], "completed");
    }
}
#[test]
fn pre_cancelled_runs_return_valid_cancelled_reports() {
    let result = super::scan(
        &request(
            json!([content("js", "https://example.test/a.js", "globalThis.x=1")]),
            json!({}),
        ),
        Arc::new(AtomicBool::new(true)),
    )
    .unwrap();
    let report: Value = serde_json::from_str(&result).unwrap();
    validate(&report).unwrap();
    assert_eq!(report["runStatus"], "cancelled");
    assert!(has_reason(&report, "cancelled"));
    assert!(report["resources"].as_array().unwrap().is_empty());
}
#[test]
fn access_policy_checks_protocol_host_port_and_canonical_roots() {
    assert_eq!(
        authorize("data:text/plain,x", &json!({})).unwrap_err(),
        "protocol-not-allowed"
    );
    assert_eq!(
        authorize("https://user:secret@example.test/", &json!({})).unwrap_err(),
        "url-credentials-prohibited"
    );
    assert_eq!(
        authorize(
            "https://example.test:8443/",
            &json!({"access":{"hosts":["example.test"]}})
        )
        .unwrap_err(),
        "host-not-allowed"
    );
    assert!(
        authorize(
            "https://example.test:8443/",
            &json!({"access":{"hosts":["example.test:8443"]}})
        )
        .is_ok()
    );
    assert_eq!(
        authorize(
            "https://example.test/",
            &json!({"access":{"protocols":["file:"]}})
        )
        .unwrap_err(),
        "protocol-not-allowed"
    );
    let dir = TempDir::new();
    let inside = dir.write("allowed/a.js", "");
    let outside = dir.write("outside.js", "");
    let options = json!({"access":{"fileRoots":[dir.0.join("allowed")]}});
    assert!(authorize(Url::from_file_path(&inside).unwrap().as_str(), &options).is_ok());
    assert_eq!(
        authorize(Url::from_file_path(&outside).unwrap().as_str(), &options).unwrap_err(),
        "file-outside-allowed-roots"
    );
}
#[test]
fn file_loading_enforces_utf8_and_byte_limits() {
    let dir = TempDir::new();
    let file = dir.write("a.js", "globalThis.x=1");
    let bad = dir.write("bad.js", [255]);
    let flag = AtomicBool::new(false);
    let deadline = now() + 10000;
    let source = Source::File {
        path: file.to_string_lossy().into(),
    };
    let loaded = load(&source, &json!({}), &Budgets::default(), &flag, deadline).unwrap();
    assert_eq!(loaded.content, "globalThis.x=1");
    let small = Budgets {
        max_resource_bytes: 1,
        ..Budgets::default()
    };
    assert_eq!(
        load(&source, &json!({}), &small, &flag, deadline)
            .err()
            .unwrap(),
        "resource-byte-budget"
    );
    assert_eq!(
        load(
            &Source::File {
                path: bad.to_string_lossy().into()
            },
            &json!({}),
            &Budgets::default(),
            &flag,
            deadline
        )
        .err()
        .unwrap(),
        "invalid-utf8"
    );
}
#[test]
fn bounded_reads_stop_at_limit_cancellation_or_deadline() {
    let flag = AtomicBool::new(false);
    assert_eq!(
        read_bounded(&b"abc"[..], 3, &flag, now() + 10000).unwrap(),
        b"abc"
    );
    assert_eq!(
        read_bounded(&b"abcd"[..], 3, &flag, now() + 10000).unwrap_err(),
        "resource-byte-budget"
    );
    assert_eq!(
        read_bounded(&b"a"[..], 3, &flag, 0).unwrap_err(),
        "total-time-budget"
    );
    assert_eq!(
        read_bounded(&b"a"[..], 3, &AtomicBool::new(true), now() + 10000).unwrap_err(),
        "cancelled"
    );
}
#[test]
fn source_maps_enrich_locations_without_replacing_artifact_evidence() {
    let map = json!({"version":3,"sources":["original.ts"],"names":[],"mappings":"AAAA","sourcesContent":["globalThis.x=1"]});
    let body = base64::engine::general_purpose::STANDARD.encode(map.to_string());
    let code = format!("globalThis.x=1;\n//# sourceMappingURL=data:application/json;base64,{body}");
    let report = one(&code, json!({"sourceMaps":true}));
    assert_eq!(report["analysisInstances"][0]["sourceMapStatus"], "mapped");
    assert_eq!(
        report["globalAccesses"][0]["location"]["original"]["source"],
        "original.ts"
    );
    assert_eq!(
        report["globalAccesses"][0]["location"]["offsetEncoding"],
        "utf16"
    );
    let invalid = one(
        "globalThis.x=1;\n//# sourceMappingURL=data:application/json;base64,invalid",
        json!({"sourceMaps":true}),
    );
    assert_eq!(
        invalid["analysisInstances"][0]["sourceMapStatus"],
        "unavailable"
    );
    assert_eq!(invalid["scanStatus"], "completed");
    assert_eq!(percent_decode("a%20b%2F").unwrap(), b"a b/");
    assert!(percent_decode("%zz").is_none());
    assert!(percent_decode("%2").is_none());
}
#[test]
fn jsonl_replay_reconstructs_records_after_summary_reanalysis() {
    let dir = TempDir::new();
    let stream = dir.0.join("report.jsonl");
    let output = dir.0.join("report.json");
    let report = one(
        "import {key} from './dep.js';globalThis[key]=1",
        json!({"jsonlPath":stream,"reportPath":output,"resourceMappings":{"https://example.test/dep.js":{"type":"content","resourceUrl":"https://example.test/dep.js","content":"export const key='shared'"}}}),
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(output).unwrap()).unwrap(),
        report
    );
    let rows: Vec<Value> = fs::read_to_string(stream)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.last().unwrap()["type"], "summary");
    assert!(rows.iter().any(|r| r["type"] == "resetAnalysis"));
    let mut accesses = HashMap::<String, Value>::new();
    let mut refs = HashMap::<String, Value>::new();
    for row in rows {
        match text(&row["type"]) {
            "globalAccesses" => {
                accesses.insert(text(&row["value"]["accessId"]).into(), row["value"].clone());
            }
            "references" => {
                refs.insert(
                    text(&row["value"]["referenceId"]).into(),
                    row["value"].clone(),
                );
            }
            "resetAnalysis" => {
                accesses.retain(|_, v| v["analysisId"] != row["value"]["analysisId"]);
                refs.retain(|_, v| v["fromAnalysisId"] != row["value"]["analysisId"]);
            }
            _ => {}
        }
    }
    assert_eq!(
        accesses.len(),
        report["globalAccesses"].as_array().unwrap().len()
    );
    assert!(
        report["globalAccesses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| accesses.get(text(&v["accessId"])) == Some(v))
    );
    assert!(
        report["references"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| refs.get(text(&v["referenceId"])) == Some(v))
    );
}
#[test]
fn checkpoints_preserve_configuration_without_authentication_and_revalidate_inputs() {
    let dir = TempDir::new();
    let file = dir.write("app.js", "globalThis.before=1");
    let cp = dir.0.join("checkpoint.json");
    let entries = json!([{"kind":"js","source":{"type":"file","path":file}}]);
    scan(
        entries,
        json!({"checkpointPath":cp,"requestHeaders":{"example.test":{"Authorization":"test-only-secret"}},"proxy":"http://test-only-proxy.invalid","access":{"fileRoots":[dir.0]},"sourceMaps":true}),
    );
    let checkpoint: Value = serde_json::from_slice(&fs::read(&cp).unwrap()).unwrap();
    assert!(!checkpoint.to_string().contains("test-only-secret"));
    assert!(!checkpoint.to_string().contains("test-only-proxy"));
    assert_eq!(checkpoint["options"]["sourceMaps"], true);
    fs::write(file, "globalThis.after=2").unwrap();
    let resumed = scan(json!([]), json!({"resume":checkpoint}));
    assert!(
        resumed["globalTargets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["pathSegments"] == json!(["after"]))
    );
    assert!(
        !resumed["globalTargets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["pathSegments"] == json!(["before"]))
    );
    assert_eq!(
        error(json!([]), json!({"resume":checkpoint,"scopeKey":"changed"})),
        "checkpoint-configuration-mismatch"
    );
    let mut bad = checkpoint;
    bad["checkpointVersion"] = json!(99);
    assert_eq!(
        error(json!([]), json!({"resume":bad})),
        "unsupported-checkpoint-version"
    );
    let from_path = scan(json!([]), json!({"resumePath":cp}));
    assert_eq!(from_path["scanStatus"], "completed");
}
#[test]
fn report_validation_rejects_schema_and_dangling_relationships() {
    assert!(
        validate(&json!({}))
            .unwrap_err()
            .starts_with("invalid-report:")
    );
    let base = js_report("globalThis.x=1");
    for (collection, field, value, expected) in [
        (
            "analysisInstances",
            "resourceId",
            json!("missing"),
            "dangling-resource",
        ),
        (
            "globalAccesses",
            "analysisId",
            json!("missing"),
            "dangling-access",
        ),
        (
            "globalAccesses",
            "targetRefs",
            json!(["missing"]),
            "dangling-access",
        ),
        (
            "findings",
            "accessRefs",
            json!(["missing"]),
            "dangling-finding",
        ),
    ] {
        let mut report = base.clone();
        report[collection][0][field] = value;
        assert_eq!(validate(&report).unwrap_err(), expected);
    }
    let mut report = one("import './a.js'", json!({"dependencyMode":"single"}));
    report["references"][0]["fromAnalysisId"] = json!("missing");
    assert_eq!(validate(&report).unwrap_err(), "dangling-reference");
}
#[test]
fn json_writes_replace_files_and_report_invalid_destinations() {
    let dir = TempDir::new();
    let file = dir.write("result.json", "old");
    write_json(file.to_str().unwrap(), &json!({"new":true})).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(file).unwrap()).unwrap(),
        json!({"new":true})
    );
    assert!(
        write_json(
            dir.0.join("missing/result.json").to_str().unwrap(),
            &json!({})
        )
        .is_err()
    );
}
