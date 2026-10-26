use super::*;

#[test]
fn partial_options_keep_defaults_and_reject_bad_types() {
    let opts: Options =
        serde_json::from_value(serde_json::json!({"budgets":{"maxDepth":0}})).unwrap();
    assert_eq!(opts.budgets.max_depth, 0);
    assert_eq!(opts.budgets.max_resources, Budgets::default().max_resources);
    assert!(serde_json::from_value::<Options>(serde_json::json!({"scriptMode":"auto"})).is_err());
    assert!(
        serde_json::from_value::<Options>(serde_json::json!({"budgets":{"maxResources":-1}}))
            .is_err()
    );
}

#[test]
fn anonymous_content_identity_is_stable_and_content_based() {
    let a = Source::Content {
        content: "a".into(),
        resource_url: None,
    };
    let b = Source::Content {
        content: "b".into(),
        resource_url: None,
    };
    assert_eq!(a.url(), a.clone().url());
    assert_ne!(a.url(), b.url());
    assert!(a.url().unwrap().starts_with("memory:"));
}
