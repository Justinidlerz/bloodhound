use crate::model::*;
use crate::test_support::*;

#[test]
fn failed_parse_has_failed_status_and_a_coverage_gap() {
    let m = js("const = ;");
    assert_eq!(m.status, ScanStatus::Failed);
    assert!(has_gap(&m, "parse-error"));
    assert_eq!(m.summary.gaps, 1);
}

#[test]
fn severity_summary_and_locations_are_consistent() {
    let m = mjs("// 😀\nwindow.x=1; Array.prototype.x=1;");
    assert_eq!(m.schema_version, 2);
    assert_eq!(m.summary.errors, 1);
    assert_eq!(m.summary.warnings, 1);
    assert_eq!(m.findings[0].location.line, 2);
    assert_eq!(m.findings[0].snippet, "window.x=1");
    assert_eq!(m.sources[0].chain, m.entries);
}
