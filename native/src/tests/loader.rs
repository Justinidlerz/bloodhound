use super::*;
use crate::test_support::*;

#[test]
fn local_files_obey_roots_protocols_and_byte_limits() {
    let dir = TempDir::new();
    let file = dir.write("space name.js", "window.x=1");
    let source = Source::File {
        path: file.to_string_lossy().into_owned(),
    };
    let mut opts = options();
    assert!(
        Loader::new(&opts)
            .unwrap()
            .load(&source, &Cancellation::never())
            .unwrap_err()
            .contains("protocol-not-allowed")
    );
    opts.access.protocols = Some(vec!["file:".into()]);
    opts.access.file_roots = Some(vec![dir.0.to_string_lossy().into_owned()]);
    let loaded = Loader::new(&opts)
        .unwrap()
        .load(&source, &Cancellation::never())
        .unwrap();
    assert_eq!(loaded.final_url, dir.url("space name.js"));
    opts.budgets.max_resource_bytes = 3;
    assert_eq!(
        Loader::new(&opts)
            .unwrap()
            .load(&source, &Cancellation::never())
            .unwrap_err(),
        "resource-byte-budget"
    );
    opts.access.file_roots = Some(vec![]);
    assert_eq!(
        Loader::new(&opts)
            .unwrap()
            .load(&source, &Cancellation::never())
            .unwrap_err(),
        "file-outside-allowed-roots"
    );
}

#[test]
fn redirected_requests_are_authorized_again() {
    let other = Server::new(vec![("/target", 200, "window.x=1")]);
    let destination = format!("{}/target", other.base);
    let server = Server::new(vec![("/start", 301, &destination)]);
    let mut opts = Options::default();
    opts.access.hosts = Some(vec![server.host()]);
    let source = Source::Url {
        url: format!("{}/start", server.base),
    };
    assert_eq!(
        Loader::new(&opts)
            .unwrap()
            .load(&source, &Cancellation::never())
            .unwrap_err(),
        "host-not-allowed"
    );
}

#[test]
fn cancellation_and_bounded_reads_stop_work() {
    let cancel = Cancellation::never();
    cancel.cancel();
    assert_eq!(
        read_bounded(&b"abc"[..], 3, &cancel).unwrap_err(),
        "cancelled"
    );
    assert_eq!(
        read_bounded(&b"abcd"[..], 3, &Cancellation::never()).unwrap_err(),
        "resource-byte-budget"
    );
    assert_eq!(
        read_bounded(&b"abc"[..], 3, &Cancellation::never()).unwrap(),
        b"abc"
    );
}

#[test]
fn source_maps_can_be_supplied_offline() {
    let mut opts = options();
    opts.resource_mappings.insert(
        "https://example.test/a.js.map".into(),
        Source::Content {
            content: r#"{"version":3,"sources":["original.ts"],"names":[],"mappings":"AAAA"}"#
                .into(),
            resource_url: None,
        },
    );
    let loader = Loader::new(&opts).unwrap();
    let map = source_map(
        &loader,
        "window.x=1;\n//# sourceMappingURL=a.js.map",
        "https://example.test/a.js",
        &Cancellation::never(),
    )
    .unwrap();
    assert_eq!(
        map.lookup_token(0, 0).unwrap().get_source(),
        Some("original.ts")
    );
    assert!(
        source_map(
            &loader,
            "const fake='sourceMappingURL=a.js.map';",
            "https://example.test/a.js",
            &Cancellation::never()
        )
        .is_none()
    );
}
