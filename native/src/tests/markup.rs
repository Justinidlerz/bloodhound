use super::*;
use crate::test_support::{context, has_reason, rules};
fn parse(code: &str) -> Value {
    html(code, "https://example.test/page.html", &context(), 10000)
}
#[test]
fn html_discovers_base_script_modes_styles_and_attributes() {
    let out = parse(
        r#"<!doctype html><base href="/assets/"><base href="/ignored/"><script src="a.js" async></script><script type="module">import './b.js'</script><script nomodule src="old.js"></script><link rel="stylesheet" href="style.css"><link rel="modulepreload" href="pre.js"><style>body{color:red}</style>"#,
    );
    let deps = out["dependencies"].as_array().unwrap();
    assert_eq!(deps.len(), 6);
    assert!(
        deps.iter()
            .all(|d| d["baseUrl"] == "https://example.test/assets/")
    );
    assert_eq!(deps[0]["scriptMode"], "classic");
    assert!(deps[0]["attributes"].get("async").is_some());
    assert_eq!(deps[1]["scriptMode"], "module");
    assert_eq!(deps[1]["content"], "import './b.js'");
    assert_eq!(deps[2]["condition"], "nomodule");
    assert_eq!(deps[3]["kind"], "style");
    assert_eq!(deps[4]["kind"], "preload");
    assert_eq!(deps[5]["content"], "body{color:red}");
}
#[test]
fn html_import_maps_are_snapshots_and_late_maps_report_gaps() {
    let out = parse(
        r#"<script type="importmap">{"imports":{"x":"/one.js"}}</script><script type="module">import 'x'</script><script type="importmap">{"imports":{"x":"/two.js","y":"/y.js"}}</script><script type="module">import 'y'</script>"#,
    );
    assert_eq!(out["dependencies"].as_array().unwrap().len(), 2);
    assert_eq!(
        out["dependencies"][0]["importMap"]["imports"]["x"],
        "https://example.test/one.js"
    );
    assert!(
        out["dependencies"][0]["importMap"]["imports"]
            .get("y")
            .is_none()
    );
    assert_eq!(
        out["dependencies"][1]["importMap"]["imports"]["x"],
        "https://example.test/one.js"
    );
    assert_eq!(
        out["dependencies"][1]["importMap"]["imports"]["y"],
        "https://example.test/y.js"
    );
    assert!(has_reason(&out, "import-map-after-module-resolution"));
}
#[test]
fn html_ignores_data_blocks_comments_and_inert_templates() {
    let out = parse(
        r#"<!-- <script src="comment.js"></script> --><script type="application/json">{"fake":"<script>"}</script><template><script src="inert.js"></script></template><script src="real.js"></script>"#,
    );
    assert_eq!(out["dependencies"].as_array().unwrap().len(), 1);
    assert_eq!(out["dependencies"][0]["specifiers"], json!(["real.js"]));
}
#[test]
fn html_entities_srcdoc_and_handler_locations_preserve_uncertainty() {
    let out = parse(
        r#"<iframe src="ignored.html" srcdoc="&lt;script&gt;window.x=1&lt;/script&gt;"></iframe><button onclick="window.x=2" style="color:red">x</button>"#,
    );
    assert_eq!(
        out["dependencies"][0]["content"],
        "<script>window.x=1</script>"
    );
    assert_eq!(out["dependencies"][0]["sourceLocationApproximate"], true);
    assert!(has_reason(&out, "html-event-handler-special-scope"));
    assert!(rules(&out).contains(&"inline-style"));
}
#[test]
fn html_reports_invalid_maps_foreign_context_and_unclosed_raw_text() {
    for (code, reason) in [
        (
            "<script type=importmap>{broken}</script>",
            "invalid-import-map",
        ),
        ("<svg><script>x</script></svg>", "html-foreign-tree-context"),
        ("<noscript>x</noscript>", "html-scripting-context"),
        ("<script>window.x=1", "unclosed-html-rawtext"),
    ] {
        assert!(has_reason(&parse(code), reason), "{reason}");
    }
    let out = html(
        "<div></div><script>x</script>",
        "https://example.test/",
        &context(),
        0,
    );
    assert!(has_reason(&out, "html-step-budget"));
}
#[test]
fn css_extracts_imports_and_asset_urls_in_nested_rules() {
    let out = css(
        r#"@import "theme.css"; @import url('base.css'); @media screen { body { background:url(image.png); mask:url("mask.svg"); } }"#,
        10000,
    );
    let deps = out["dependencies"].as_array().unwrap();
    assert_eq!(deps.len(), 4);
    assert_eq!(
        deps.iter()
            .map(|d| d["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["css-import", "css-import", "css-url", "css-url"]
    );
    assert_eq!(
        deps.iter()
            .map(|d| d["specifiers"][0].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["theme.css", "base.css", "image.png", "mask.svg"]
    );
}
#[test]
fn css_distinguishes_global_selectors_from_similarly_named_classes() {
    assert!(!rules(&css(".body,.html {color:red}", 10000)).contains(&"global-selector"));
    for selector in ["body", "html", ":root", "*"] {
        assert!(
            rules(&css(&format!("{selector} {{color:red}}"), 10000)).contains(&"global-selector")
        );
    }
}
#[test]
fn css_records_reset_properties_animations_and_keyframes() {
    let out = css(
        ":root {--theme:red;all:unset;animation:spin 1s;animation-name:spin} @keyframes spin {to{opacity:0}}",
        10000,
    );
    for rule in [
        "custom-property",
        "reset-declaration",
        "animation-name",
        "keyframes-name",
    ] {
        assert!(rules(&out).contains(&rule), "{rule}");
    }
    assert!(
        out["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["details"]["certainty"] == "static-candidate")
    );
}
#[test]
fn css_invalid_urls_and_budgets_produce_gaps() {
    assert!(has_reason(&css("body{color:red}", 0), "css-step-budget"));
    assert!(has_reason(&css("@import ;", 1000), "unknown-css-import"));
    let bad = css("body{background:url(\"a\" junk)}", 1000);
    assert!(!bad["unresolved"].as_array().unwrap().is_empty());
}
