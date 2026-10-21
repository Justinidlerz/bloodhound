use super::*;
use crate::test_support::js_report;
#[test]
fn strings_are_always_content_and_explicit_sources_are_preserved() {
    for content in [
        "globalThis.x=1",
        "<script>window.x=1</script>",
        "https://example.test/a",
        "./a.js",
    ] {
        assert_eq!(
            source(json!(content)),
            json!({"type":"content","content":content})
        );
    }
    for input in [
        json!({"type":"file","path":"./a.js"}),
        json!({"type":"url","url":"https://example.test/a"}),
    ] {
        assert_eq!(source(input.clone()), input);
    }
}
#[test]
fn defaults_and_engine_versions_describe_compiled_code() {
    assert_eq!(defaults()["versions"]["scanner"], env!("CARGO_PKG_VERSION"));
    assert_eq!(defaults()["versions"]["parser"], engine_version());
    assert_eq!(
        defaults()["budgets"],
        serde_json::to_value(model::Budgets::default()).unwrap()
    );
}
#[test]
fn public_index_and_report_helpers_delegate_with_validated_inputs() {
    let report = js_report("globalThis.x=1;");
    validate_report(ReportInput(report.clone())).unwrap();
    let index = GlobalIndex::new(ReportInput(report.clone())).unwrap();
    assert_eq!(index.report().0, report);
    let targets = index
        .find_targets(vec!["x".into()], Some("global-object".into()), None)
        .unwrap()
        .0;
    let id = targets[0]["targetId"].as_str().unwrap().to_owned();
    assert_eq!(
        index
            .sources(id.clone())
            .unwrap()
            .0
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !index
            .accesses(
                report["analysisInstances"][0]["analysisId"]
                    .as_str()
                    .unwrap()
                    .into()
            )
            .unwrap()
            .0
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !index
            .resource_accesses(
                report["resources"][0]["resourceId"]
                    .as_str()
                    .unwrap()
                    .into()
            )
            .unwrap()
            .0
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        index.entry_paths(id.clone(), None).unwrap().0["paths"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(index.entry_paths(id, Some(json!([]))).is_err());
    assert_eq!(index.conflicts().unwrap().0, json!([]));
    assert_eq!(index.resolve_unbound().unwrap().0, json!([]));
    assert_eq!(
        diff_reports(ReportInput(report.clone()), ReportInput(report.clone()))
            .unwrap()
            .0["changed"],
        json!([])
    );
    assert_eq!(
        index_artifact(ReportInput(report.clone()), None).unwrap().0["reportId"],
        report["scanId"]
    );
    assert_eq!(
        adapt_report(ReportInput(report.clone()), "page".into(), None)
            .unwrap()
            .0["reportType"],
        "page"
    );
    assert!(
        !runtime_candidates(ReportInput(report), json!({"pathSegments":["x"]}))
            .unwrap()
            .0
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn cancellation_handles_are_shared_per_run_and_removed_on_drop() {
    let id = format!("unit-{}", NEXT_RUN.fetch_add(1, Ordering::Relaxed));
    let flag = cancellation(&id);
    let other = cancellation(&id);
    assert!(Arc::ptr_eq(&flag, &other));
    flag.store(true, Ordering::Relaxed);
    assert!(other.load(Ordering::Relaxed));
    drop(ScanTask {
        request: String::new(),
        id: id.clone(),
        flag,
        listener: None,
    });
    assert!(!RUNS.get().unwrap().lock().unwrap().contains_key(&id));
}
