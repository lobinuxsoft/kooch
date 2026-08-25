use std::time::Duration;

use super::{ScriptSync, SyncState, fingerprint};

/// A directory of its own, the way the rest of this crate does it —
/// there is no `tempfile` in this workspace.
fn src_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_script_sync_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    dir
}

fn write(dir: &std::path::Path, name: &str, body: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, body).expect("write");
}

/// A deleted file moves the fingerprint even though no mtime grew.
///
/// 🔴 This is why the count is there. Removing the newest file leaves an
/// older maximum behind, so mtime alone reports "nothing has happened
/// since" while a system just stopped existing — and the registration
/// for it would keep naming a module that is gone, which does not run,
/// it fails to compile.
#[test]
fn a_deleted_file_moves_the_fingerprint() {
    let dir = src_dir("deleted");
    write(&dir, "one.rs", "pub fn one() {}");
    write(&dir, "two.rs", "pub fn two() {}");
    let before = fingerprint(&dir).expect("fingerprint");

    std::fs::remove_file(dir.join("two.rs")).expect("remove");
    let after = fingerprint(&dir).expect("fingerprint");

    assert_ne!(
        before, after,
        "the fingerprint did not move, so a removed system would go unnoticed \
         until something else in src/ happened to be saved"
    );
}

/// Writing the generated file does not itself look like a change.
///
/// Without the exclusion every regeneration would move the fingerprint,
/// scheduling a scan that reads every file in the project to conclude
/// nothing needs doing — on a FUSE mount, after every save.
#[test]
fn the_generated_file_is_not_a_change() {
    let dir = src_dir("generated");
    write(&dir, "one.rs", "pub fn one() {}");
    let before = fingerprint(&dir).expect("fingerprint");

    write(&dir, "registrations.rs", "// AUTO-GENERATED\n");
    let after = fingerprint(&dir).expect("fingerprint");

    assert_eq!(
        before, after,
        "`registrations.rs` counted towards the fingerprint, so writing it \
         schedules a scan that can only find nothing"
    );
}

/// Nested directories are seen. A project puts systems in `src/systems/`.
#[test]
fn a_nested_file_is_counted() {
    let dir = src_dir("nested");
    write(&dir, "one.rs", "pub fn one() {}");
    let flat = fingerprint(&dir).expect("fingerprint");

    write(&dir, "systems/spin.rs", "pub fn spin() {}");
    let nested = fingerprint(&dir).expect("fingerprint");

    assert_eq!(flat.1 + 1, nested.1, "the walk stopped at the top level");
}

/// Non-Rust files are ignored, so saving an asset beside the code does
/// not announce a rebuild nobody needs.
#[test]
fn only_rust_counts() {
    let dir = src_dir("only_rust");
    write(&dir, "one.rs", "pub fn one() {}");
    let before = fingerprint(&dir).expect("fingerprint");

    write(&dir, "notes.md", "# not code");
    assert_eq!(before, fingerprint(&dir).expect("fingerprint"));
}

/// Acknowledging clears the warning, and nothing else does.
#[test]
fn acknowledging_clears_the_warning() {
    let mut sync = ScriptSync {
        state: SyncState::NeedsRebuild,
        fingerprint: Some((Duration::ZERO, 1)),
        ..Default::default()
    };
    sync.acknowledge();
    assert_eq!(sync.state, SyncState::Current);
}
