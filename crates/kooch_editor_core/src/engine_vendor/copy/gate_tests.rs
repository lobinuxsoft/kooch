use super::gates_on_test;

#[test]
fn a_plain_cfg_test_gates() {
    assert!(gates_on_test("#[cfg(test)]"));
}

#[test]
fn a_feature_gated_test_gates() {
    // What shipped a vendored engine that would not compile.
    assert!(gates_on_test("#[cfg(all(test, feature = \"physics\"))]"));
}

#[test]
fn any_test_does_not_gate() {
    // `any(test, …)` still compiles without tests.
    assert!(!gates_on_test("#[cfg(any(test, feature = \"physics\"))]"));
}

#[test]
fn not_test_does_not_gate() {
    assert!(!gates_on_test("#[cfg(not(test))]"));
}

#[test]
fn a_feature_alone_does_not_gate() {
    assert!(!gates_on_test("#[cfg(feature = \"physics\")]"));
}
