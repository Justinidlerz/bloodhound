use super::*;

#[test]
fn candidate_union_is_finite_and_unknown_is_conservative() {
    let a = Value::Strings(vec!["a".into()]);
    let b = Value::Strings(vec!["b".into()]);
    assert_eq!(a.clone().join(b.clone(), 2).strings(), ["a", "b"]);
    assert_eq!(a.clone().join(b, 1), Value::Unknown);
    assert_eq!(a.join(Value::Unknown, 2), Value::Unknown);
}
