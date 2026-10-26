use super::*;
use crate::model::*;
use crate::test_support::*;

#[test]
fn css_is_discovered_but_only_document_inline_styles_are_findings() {
    let html = r#"<html style="color:red"><body style="margin:0"><div style="width:1px"></div><link rel="stylesheet" href="a.css"><style>:root{--theme:red} @keyframes x{}</style><script type="application/json">{"x":"<script>"}</script><template><script>window.inert=1</script></template><script>window.active=1</script>"#;
    let m = manifest(
        vec![content(Kind::Html, "https://example.test/", html)],
        options(),
    );
    assert_eq!(m.findings.len(), 3);
    assert_eq!(
        m.findings
            .iter()
            .filter(|f| f.rule == Rule::HtmlGlobalStyle)
            .count(),
        2
    );
    assert_eq!(
        m.dependencies
            .iter()
            .filter(|d| d.kind == DependencyKind::Style)
            .count(),
        2
    );
    assert!(
        m.dependencies
            .iter()
            .filter(|d| d.kind == DependencyKind::Style)
            .all(|d| d.reason.as_deref() == Some("css-not-analyzed"))
    );
    assert_eq!(m.sources.len(), 2);
    assert!(targets(&m).contains(&"window.active"));
}

#[test]
fn inline_spans_preserve_exact_source_and_utf16_origin() {
    let html = "😀\n<p>x</p><script>\nwindow.x = 1;\n</script>";
    let a = extract(html, "https://example.test/", 1000);
    let dep = &a.dependencies[0];
    assert_eq!(dep.content.as_deref(), Some("\nwindow.x = 1;\n"));
    assert_eq!(
        dep.content_offset.unwrap() as usize,
        html.find("\nwindow").unwrap()
    );
    let m = manifest(
        vec![content(Kind::Html, "https://example.test/", html)],
        options(),
    );
    let origin = m.sources[1].inline.as_ref().unwrap();
    assert_eq!((origin.line, origin.column), (2, 16));
    assert_eq!(
        (m.findings[0].location.line, m.findings[0].location.column),
        (2, 0)
    );
}

#[test]
fn base_import_map_and_single_mode_are_explicit() {
    let html = r#"<base href="/assets/"><script type="importmap">{"imports":{"pkg":"./pkg.js"}}</script><script type="module">import 'pkg';</script>"#;
    let a = extract(html, "https://example.test/page", 1000);
    assert_eq!(a.base_url.as_deref(), Some("https://example.test/assets/"));
    assert_eq!(
        a.import_map.unwrap().imports["pkg"].as_deref(),
        Some("https://example.test/assets/pkg.js")
    );
    let mut opts = options();
    opts.dependency_mode = DependencyMode::Single;
    let m = manifest(
        vec![content(Kind::Html, "https://example.test/", html)],
        opts,
    );
    assert_eq!(m.sources.len(), 1);
    assert!(
        m.dependencies
            .iter()
            .all(|d| d.reason.as_deref() == Some("single-mode"))
    );
}

#[test]
fn unsupported_html_contexts_and_late_maps_are_visible_gaps() {
    let a = extract(
        r#"<svg><script>window.x=1</script></svg><script type="module">import 'pkg'</script><script type="importmap">{"imports":{"pkg":"/pkg.js"}}</script>"#,
        "https://example.test/",
        1000,
    );
    assert!(a.gaps.iter().any(|g| g.reason == "foreign-html-content"));
    assert!(a.gaps.iter().any(|g| g.reason == "late-import-map"));
    assert!(a.import_map.is_none());
}
