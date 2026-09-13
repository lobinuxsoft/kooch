//! Test code for `assets`, in its own file.

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
