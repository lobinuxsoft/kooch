use super::*;

#[test]
fn rejects_absolute_posix() {
    assert_eq!(reject_unsafe_uri("/etc/passwd"), Some("absolute path"));
}

#[test]
fn rejects_absolute_windows() {
    assert_eq!(
        reject_unsafe_uri("C:/Windows/System32"),
        Some("absolute path")
    );
    assert_eq!(reject_unsafe_uri("C:\\Windows"), Some("absolute path"));
    assert_eq!(
        reject_unsafe_uri("\\\\server\\share"),
        Some("absolute path")
    );
}

#[test]
fn rejects_dot_dot_traversal() {
    assert_eq!(
        reject_unsafe_uri("../etc/passwd"),
        Some("`..` traversal not allowed"),
    );
    assert_eq!(
        reject_unsafe_uri("safe/..\\sneaky"),
        Some("`..` traversal not allowed"),
    );
}

#[test]
fn rejects_uri_schemes() {
    assert_eq!(
        reject_unsafe_uri("file:///etc/passwd"),
        Some("uri scheme not allowed"),
    );
    assert_eq!(
        reject_unsafe_uri("http://evil.example/x.bin"),
        Some("uri scheme not allowed"),
    );
}

#[test]
fn accepts_simple_relative_names() {
    assert_eq!(reject_unsafe_uri("scene.bin"), None);
    assert_eq!(reject_unsafe_uri("buffers/data.bin"), None);
}
