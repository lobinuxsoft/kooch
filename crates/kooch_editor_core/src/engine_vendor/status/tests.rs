use std::fs;
use std::path::{Path, PathBuf};

use super::super::ensure_current_in;
use super::{Difference, status_in};

/// A directory with the shape of the engine, and `body` in it so two
/// calls can produce trees that differ.
fn fake_engine(root: &Path, body: &str) {
    fs::create_dir_all(root.join("crates/kooch_core/src")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]").unwrap();
    fs::write(root.join("src/lib.rs"), body).unwrap();
    // 🔴 A file, not just the directory: the walk copies files, so an
    // empty `crates/` never reaches the destination and what lands there
    // does not pass `is_engine_source`.
    fs::write(root.join("crates/kooch_core/src/lib.rs"), "// core").unwrap();
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_status_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn nothing_installed_is_absent() {
    let dir = tmp("absent");
    let source = dir.join("source");
    fake_engine(&source, "// engine");

    assert_eq!(
        status_in(&dir.join("installed"), Some(&source)),
        Difference::Absent,
    );
}

#[test]
fn what_this_editor_installed_is_current() {
    let dir = tmp("current");
    let (source, dest) = (dir.join("source"), dir.join("installed"));
    fake_engine(&source, "// engine");
    ensure_current_in(&dest, Some(&source)).expect("installs");

    assert_eq!(status_in(&dest, Some(&source)), Difference::Current);
}

/// 🔴 The case the whole thing exists for. `CARGO_PKG_VERSION` does not
/// move during development, so the version says these are the same
/// engine and only the tree hash disagrees — which is exactly the
/// situation a person needs told, and the one that silently replaced a
/// project's engine before.
#[test]
fn a_rebuilt_engine_is_not_current() {
    let dir = tmp("rebuilt");
    let (source, dest) = (dir.join("source"), dir.join("installed"));
    fake_engine(&source, "// engine");
    ensure_current_in(&dest, Some(&source)).expect("installs");

    // The engine is worked on. Same version, different tree.
    fake_engine(&source, "// engine, with today's work");

    assert_eq!(status_in(&dest, Some(&source)), Difference::Rebuilt);
}

/// Looking must not install. The old behaviour was to materialise while
/// answering, which is what made the first rebuild after opening a
/// project a full one nobody asked for.
#[test]
fn asking_changes_nothing_on_disk() {
    let dir = tmp("read_only");
    let (source, dest) = (dir.join("source"), dir.join("installed"));
    fake_engine(&source, "// engine");
    ensure_current_in(&dest, Some(&source)).expect("installs");
    fake_engine(&source, "// engine, rebuilt");

    let before = fs::read_to_string(dest.join("src/lib.rs")).unwrap();
    status_in(&dest, Some(&source));
    let after = fs::read_to_string(dest.join("src/lib.rs")).unwrap();

    assert_eq!(before, after, "status wrote to the installed engine");
    assert_eq!(after, "// engine", "the installed engine was replaced");
}

/// A packaged editor with no source beside it, against an engine that is
/// already there: nothing to compare, and the project still builds.
#[test]
fn no_source_leaves_what_is_installed_alone() {
    let dir = tmp("no_source");
    let (source, dest) = (dir.join("source"), dir.join("installed"));
    fake_engine(&source, "// engine");
    ensure_current_in(&dest, Some(&source)).expect("installs");

    assert_eq!(status_in(&dest, None), Difference::Current);
    assert_eq!(status_in(&dir.join("nowhere"), None), Difference::NoSource);
}

/// What the panel asks to decide whether to interrupt anyone.
#[test]
fn only_a_real_difference_asks() {
    assert!(!Difference::Current.wants_a_decision());
    assert!(!Difference::Absent.wants_a_decision());
    assert!(!Difference::NoSource.wants_a_decision());
    assert!(Difference::Rebuilt.wants_a_decision());
    assert!(Difference::OtherVersion.wants_a_decision());
}
