//! Test code for `assets`, in its own file.
//!
//! # 🔴 A sibling file, not an inline `mod`
//!
//! The engine vendors its own source into every project, and the
//! walk that copies it skips test code by FILE — it can drop
//! `x_tests.rs` and it cannot reach inside a module written in
//! line. An inline block therefore ships to every game that ever
//! builds against this engine, which is what
//! `the_vendored_engine_contains_no_test_code` is there to catch.

use super::needs_write;

fn scratch(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("kooch_write_guard_{name}"));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn identical_bytes_need_no_write() {
    let path = scratch("same");
    std::fs::write(&path, "(vsync: false)").unwrap();
    assert!(!needs_write(&path, "(vsync: false)"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_changed_value_needs_the_write() {
    let path = scratch("changed");
    std::fs::write(&path, "(vsync: false)").unwrap();
    assert!(needs_write(&path, "(vsync: true)"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_missing_file_needs_the_write() {
    assert!(needs_write(&scratch("absent"), "(vsync: true)"));
}
