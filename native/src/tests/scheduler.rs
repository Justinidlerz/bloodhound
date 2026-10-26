use crate::model::*;
use crate::test_support::*;

fn mapped(options: &mut Options, url: &str, code: &str) {
    options.resource_mappings.insert(
        url.into(),
        Source::Content {
            content: code.into(),
            resource_url: Some(url.into()),
        },
    );
}

#[test]
fn duplicate_entries_and_cycles_are_analyzed_once() {
    let entry = content(
        Kind::Js,
        "https://example.test/a.js",
        "import './b.js'; window.a=1;",
    );
    let mut opts = options();
    opts.script_mode = ScriptMode::Module;
    mapped(
        &mut opts,
        "https://example.test/b.js",
        "import './a.js'; window.b=1;",
    );
    let m = manifest(vec![entry.clone(), entry], opts);
    assert_eq!(m.sources.len(), 2);
    assert_eq!(m.findings.len(), 2);
    assert_eq!(m.entries[0], m.entries[1]);
    assert_eq!(m.status, ScanStatus::Completed);
}

#[test]
fn follows_every_finite_dynamic_import_candidate() {
    let mut opts = options();
    opts.script_mode = ScriptMode::Module;
    mapped(&mut opts, "https://example.test/a.js", "window.a=1");
    mapped(&mut opts, "https://example.test/b.js", "window.b=1");
    let m = manifest(
        vec![content(
            Kind::Js,
            "https://example.test/root.js",
            "import(flag ? './a.js' : './b.js');",
        )],
        opts,
    );
    assert_eq!(m.dependencies.len(), 2);
    assert_eq!(m.findings.len(), 2);
    assert_eq!(m.status, ScanStatus::Completed);
}

#[test]
fn identical_code_keeps_url_specific_import_meta_resolution() {
    let mut opts = options();
    opts.script_mode = ScriptMode::Module;
    mapped(&mut opts, "https://example.test/a/child.js", "window.a=1");
    mapped(&mut opts, "https://example.test/b/child.js", "window.b=1");
    let code = "import(new URL('./child.js', import.meta.url));";
    let m = manifest(
        vec![
            content(Kind::Js, "https://example.test/a/root.js", code),
            content(Kind::Js, "https://example.test/b/root.js", code),
        ],
        opts,
    );
    assert_eq!(m.findings.len(), 2);
    assert_eq!(m.status, ScanStatus::Completed);
}

#[test]
fn resource_and_depth_budgets_include_inline_resources() {
    let html = "<script>window.a=1</script><script>window.b=1</script>";
    let mut opts = options();
    opts.budgets.max_resources = 2;
    let m = manifest(
        vec![content(Kind::Html, "https://example.test/", html)],
        opts,
    );
    assert_eq!(m.sources.len(), 2);
    assert!(
        m.dependencies
            .iter()
            .any(|d| d.reason.as_deref() == Some("resource-count-budget"))
    );
    let mut opts = options();
    opts.budgets.max_depth = 0;
    let m = manifest(
        vec![content(Kind::Html, "https://example.test/", html)],
        opts,
    );
    assert_eq!(m.sources.len(), 1);
    assert!(
        m.dependencies
            .iter()
            .all(|d| d.reason.as_deref() == Some("depth-budget"))
    );
}

#[test]
fn distinct_documents_keep_import_maps_separate() {
    let mut opts = options();
    mapped(&mut opts, "https://example.test/shared.js", "import 'lib';");
    mapped(&mut opts, "https://example.test/a.js", "window.a=1");
    mapped(&mut opts, "https://example.test/b.js", "window.b=1");
    let doc = |target: &str| {
        format!(
            r#"<script type="importmap">{{"imports":{{"lib":"/{target}.js"}}}}</script><script type="module" src="/shared.js"></script>"#
        )
    };
    let m = manifest(
        vec![
            content(Kind::Html, "https://example.test/one", &doc("a")),
            content(Kind::Html, "https://example.test/two", &doc("b")),
        ],
        opts,
    );
    assert_eq!(m.status, ScanStatus::Completed);
    assert_eq!(m.findings.len(), 2);
}

#[test]
fn redirect_changes_relative_import_base() {
    let server = Server::new(vec![
        ("/entry.js", 301, "/nested/main.js"),
        ("/nested/main.js", 200, "import './child.js'"),
        ("/nested/child.js", 200, "window.child=1"),
    ]);
    let mut opts = Options::default();
    opts.script_mode = ScriptMode::Module;
    let entry = Entry {
        kind: Kind::Js,
        source: Source::Url {
            url: format!("{}/entry.js", server.base),
        },
        resource_url: None,
        script_mode: None,
    };
    let m = manifest(vec![entry], opts);
    assert_eq!(m.status, ScanStatus::Completed);
    assert_eq!(targets(&m), ["window.child"]);
    assert!(
        m.sources[0]
            .url
            .as_ref()
            .unwrap()
            .ends_with("/nested/main.js")
    );
}

#[test]
fn parse_failures_and_workers_report_incomplete_coverage() {
    let mut opts = options();
    opts.script_mode = ScriptMode::Module;
    mapped(&mut opts, "https://example.test/bad.js", "const = ;");
    let m = manifest(
        vec![content(
            Kind::Js,
            "https://example.test/root.js",
            "import './bad.js'; new Worker('./worker.js');",
        )],
        opts,
    );
    assert_eq!(m.status, ScanStatus::Partial);
    assert_eq!(m.summary.unresolved, 2);
    assert!(
        m.dependencies
            .iter()
            .any(|d| d.reason.as_deref() == Some("parse-error"))
    );
    assert!(
        m.dependencies
            .iter()
            .any(|d| d.reason.as_deref() == Some("worker-environment-not-analyzed"))
    );
}

#[test]
fn anonymous_inputs_use_explicit_base_and_modules_keep_document_base() {
    let mut opts = options();
    opts.script_mode = ScriptMode::Module;
    opts.base_url = Some("https://example.test/assets/".into());
    mapped(
        &mut opts,
        "https://example.test/assets/child.js",
        "window.child=1",
    );
    let entry = Entry {
        kind: Kind::Js,
        source: Source::Content {
            content: "import './child.js'".into(),
            resource_url: None,
        },
        resource_url: None,
        script_mode: None,
    };
    assert_eq!(manifest(vec![entry], opts).status, ScanStatus::Completed);

    let mut opts = options();
    opts.base_url = Some("https://example.test/assets/".into());
    mapped(
        &mut opts,
        "https://example.test/assets/preload.js",
        "window.preload=1",
    );
    let entry = Entry {
        kind: Kind::Html,
        source: Source::Content {
            content: "<link rel='modulepreload' href='preload.js'>".into(),
            resource_url: None,
        },
        resource_url: None,
        script_mode: None,
    };
    assert_eq!(manifest(vec![entry], opts).status, ScanStatus::Completed);

    let mut opts = options();
    mapped(
        &mut opts,
        "https://example.test/scripts/main.js",
        "import './nested/child.js'",
    );
    mapped(
        &mut opts,
        "https://example.test/scripts/nested/child.js",
        "const s=document.createElement('script'); s.src='dynamic.js'; document.head.appendChild(s);",
    );
    mapped(
        &mut opts,
        "https://example.test/page/dynamic.js",
        "window.dynamic=1",
    );
    let m = manifest(
        vec![content(
            Kind::Html,
            "https://example.test/page/index.html",
            "<script type='module' src='/scripts/main.js'></script>",
        )],
        opts,
    );
    assert_eq!(m.status, ScanStatus::Completed, "{:?}", m.dependencies);
    assert!(targets(&m).contains(&"window.dynamic"));
}
