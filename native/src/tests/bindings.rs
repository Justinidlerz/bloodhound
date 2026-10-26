use super::*;
use crate::model::*;
use crate::test_support::*;

#[test]
fn json_boundary_returns_v2_and_validates_requests() {
    let request = request_json(
        vec![content(Kind::Js, "https://example.test/a.js", "window.x=1")],
        options(),
    );
    let result = scan_json(&request, Arc::new(AtomicBool::new(false))).unwrap();
    let manifest: Manifest = serde_json::from_str(&result).unwrap();
    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.findings.len(), 1);
    assert!(scan_json("{}", Arc::new(AtomicBool::new(false))).is_err());
    assert!(
        scan_json(
            &request_json(vec![], options()),
            Arc::new(AtomicBool::new(false))
        )
        .is_err()
    );
    let cancelled: Manifest =
        serde_json::from_str(&scan_json(&request, Arc::new(AtomicBool::new(true))).unwrap())
            .unwrap();
    assert_eq!(cancelled.status, ScanStatus::Cancelled);
}
