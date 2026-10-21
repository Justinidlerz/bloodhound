use super::*;
use crate::test_support::context;

#[test]
fn browser_urls_resolve_without_node_extension_search() {
    let ctx = context();
    for (spec, expected) in [
        ("./dep", "https://example.test/app/dep"),
        ("../x.js", "https://example.test/x.js"),
        ("/x.js?q=1#f", "https://example.test/x.js?q=1#f"),
        ("https://cdn.test/a.js", "https://cdn.test/a.js"),
    ] {
        assert_eq!(
            resolve(spec, "https://example.test/app/main.js", &ctx, true).unwrap(),
            expected
        );
    }
    assert_eq!(
        resolve("pkg", "https://example.test/app.js", &ctx, true).unwrap_err(),
        "unmapped-bare-specifier"
    );
    assert_eq!(
        resolve("pkg", "https://example.test/app.js", &ctx, false).unwrap(),
        "https://example.test/pkg"
    );
    assert!(join("not a url", "./x").is_err());
}
#[test]
fn import_map_addresses_and_scope_keys_use_the_map_base() {
    let normalized=normalize(&json!({"imports":{"pkg":"./lib.js","./old.js":"/new.js","blocked":null},"scopes":{"./app/":{"pkg":"./special.js"}}}),"https://example.test/maps/map.json").unwrap();
    assert_eq!(
        normalized["imports"]["pkg"],
        "https://example.test/maps/lib.js"
    );
    assert_eq!(
        normalized["imports"]["https://example.test/maps/old.js"],
        "https://example.test/new.js"
    );
    assert_eq!(
        normalized["scopes"]["https://example.test/maps/app/"]["pkg"],
        "https://example.test/maps/special.js"
    );
    assert!(normalized["imports"]["blocked"].is_null());
}
#[test]
fn malformed_import_maps_are_rejected() {
    for map in [
        json!({"imports":[]}),
        json!({"imports":{"x":"bare"}}),
        json!({"imports":{"x":3}}),
        json!({"imports":{"pkg/":"./file.js"}}),
        json!({"scopes":[]}),
    ] {
        assert!(normalize(&map, "https://example.test/").is_err(), "{map}");
    }
}
#[test]
fn scopes_match_the_importer_and_fall_back_from_longest_prefix() {
    let mut ctx = context();
    ctx.import_map_base_url = Some("https://example.test/".into());
    ctx.import_map = Some(
        json!({"imports":{"pkg":"/root.js","global":"/global.js"},"scopes":{"/app/":{"pkg":"/scoped.js"},"/app/deep/":{"other":"/other.js"}}}),
    );
    assert_eq!(
        resolve("pkg", "https://example.test/app/deep/main.js", &ctx, true).unwrap(),
        "https://example.test/scoped.js"
    );
    assert_eq!(
        resolve("pkg", "https://example.test/outside.js", &ctx, true).unwrap(),
        "https://example.test/root.js"
    );
    assert_eq!(
        resolve("global", "https://example.test/app/main.js", &ctx, true).unwrap(),
        "https://example.test/global.js"
    );
}
#[test]
fn exact_and_prefix_mappings_reject_blocks_and_backtracking() {
    let mut ctx = context();
    ctx.import_map_base_url = Some("https://example.test/".into());
    ctx.import_map =
        Some(json!({"imports":{"pkg/":"/v1/","pkg/special":"/special.js","blocked":null}}));
    assert_eq!(
        resolve("pkg/a.js", "https://example.test/main.js", &ctx, true).unwrap(),
        "https://example.test/v1/a.js"
    );
    assert_eq!(
        resolve("pkg/special", "https://example.test/main.js", &ctx, true).unwrap(),
        "https://example.test/special.js"
    );
    assert_eq!(
        resolve("blocked", "https://example.test/main.js", &ctx, true).unwrap_err(),
        "import-map-blocked"
    );
    assert_eq!(
        resolve("pkg/../escape", "https://example.test/main.js", &ctx, true).unwrap_err(),
        "import-map-prefix-backtracking"
    );
}
#[test]
fn later_maps_add_keys_without_replacing_prior_mappings() {
    let prior = json!({"imports":{"a":"https://example.test/old"},"scopes":{"https://example.test/app/":{"a":null}}});
    let next = json!({"imports":{"a":"https://example.test/new","b":"https://example.test/b"},"scopes":{"https://example.test/app/":{"a":"https://example.test/new","b":"https://example.test/b"}}});
    let merged = merge_maps(&prior, &next);
    assert_eq!(merged["imports"]["a"], prior["imports"]["a"]);
    assert_eq!(merged["imports"]["b"], next["imports"]["b"]);
    assert!(merged["scopes"]["https://example.test/app/"]["a"].is_null());
}
#[test]
fn sanitization_removes_userinfo_and_kind_dispatch_handles_queries() {
    assert_eq!(
        sanitize_url("https://user:secret@example.test/a?q=1"),
        "https://example.test/a?q=1"
    );
    assert_eq!(sanitize_url("relative.js"), "relative.js");
    for (dependency, url, expected) in [
        ("iframe", "https://example.test/", "html"),
        ("style", "https://example.test/a", "css"),
        ("css-import", "https://example.test/a", "css"),
        ("css-url", "https://example.test/a", "asset"),
        ("preload", "https://example.test/a.css?v=1", "css"),
        ("import", "https://example.test/a.js", "js"),
    ] {
        assert_eq!(kind(&json!({"kind":dependency}), url), expected);
    }
}
