use super::*;

#[test]
fn known_globals_are_classified_and_locals_are_not() {
    assert_eq!(classify("window"), Some(Root::Global));
    assert_eq!(classify("Array"), Some(Root::Builtin("Array")));
    assert_eq!(classify("fetch"), Some(Root::Host("fetch")));
    assert_eq!(classify("myLocal"), None);
}
