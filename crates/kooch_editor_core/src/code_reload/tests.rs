use std::time::Duration;

use kooch_core::resource::Resources;

use super::{CodeReload, reload_code_system, stamp};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_code_reload_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    dir
}

/// A rebuild that changes the library's length moves the stamp, which is
/// the whole trigger.
#[test]
fn a_rebuilt_library_moves() {
    let dir = scratch("rebuilt");
    let lib = dir.join("libgame.so");
    std::fs::write(&lib, b"one").expect("write");
    let before = stamp(&lib).expect("stamp");

    std::fs::write(&lib, b"one and a half").expect("rebuild");
    let after = stamp(&lib).expect("stamp");

    assert_ne!(before, after, "a rebuild went unnoticed");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 🔴 Size is in the stamp because mtime alone can stand still: a build
/// finishing inside the same clock tick reads as nothing having
/// happened. The reverse case — same size, new bytes — is why mtime is
/// there too. Neither is enough alone.
#[test]
fn the_stamp_carries_both() {
    let dir = scratch("both");
    let lib = dir.join("libgame.so");
    std::fs::write(&lib, b"one").expect("write");
    let (time, size) = stamp(&lib).expect("stamp");

    assert_eq!(size, 3);
    assert!(time > Duration::ZERO, "no modification time was read");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A missing library is the ordinary state of a project that has never
/// been built, and it must not be an error every second.
#[test]
fn a_missing_library_is_silent() {
    assert_eq!(stamp(&scratch("missing").join("nothing.so")), None);
}

/// With no project open there is nothing to stat, and the poll must not
/// panic reaching for one.
#[test]
fn no_project_reloads_nothing() {
    let mut resources = Resources::new();
    resources.insert(CodeReload::default());

    reload_code_system(&mut resources);

    let reload = resources.get::<CodeReload>().expect("resource");
    assert!(
        reload.stamp.is_none(),
        "stamped a library that is not there"
    );
}
