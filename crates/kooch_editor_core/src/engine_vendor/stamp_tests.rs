//! #761 — a materialised engine has to say *which* source it came from,
//! not merely that it looks like an engine.

use super::stamp::{Check, EngineStamp, STAMP_FILE};
use super::*;

/// Same fixture shape as the sibling suite: enough to pass
/// `is_engine_source`, with build output to be skipped.
fn fake_engine(root: &Path) {
    fs::create_dir_all(root.join("crates/kooch_core/src")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("assets/materials")).unwrap();
    fs::create_dir_all(root.join("target/debug")).unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]").unwrap();
    fs::write(root.join("Cargo.lock"), "# lock").unwrap();
    fs::write(root.join("src/lib.rs"), "// facade").unwrap();
    fs::write(root.join("crates/kooch_core/src/lib.rs"), "// core").unwrap();
    fs::write(root.join("assets/materials/default.material"), "()").unwrap();
    fs::write(root.join("target/debug/huge"), vec![0u8; 4096]).unwrap();
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_stamp_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// 🔴 The property the whole mechanism rests on. The stamp is written
/// from the *source* and later read from the *copy*, so if those two
/// hashed differently every open would re-materialise — the engine would
/// be re-copied forever and never report itself current.
#[test]
fn a_copy_hashes_the_same_as_its_source() {
    let dir = tmp("copy_equals_source");
    let (engine, project) = (dir.join("engine_src"), dir.join("proj"));
    fake_engine(&engine);
    fs::create_dir_all(&project).unwrap();

    let dest = vendor_engine(&project, &engine).expect("vendors");

    assert_eq!(
        EngineStamp::of_tree(&engine).unwrap(),
        EngineStamp::of_tree(&dest).unwrap(),
        "the copy does not hash to what it was copied from",
    );
}

/// The stamp file cannot be part of what it describes, or the digest
/// would depend on itself: writing it would change the answer.
#[test]
fn the_stamp_is_not_part_of_the_tree_it_describes() {
    let dir = tmp("self_reference");
    let engine = dir.join("engine_src");
    fake_engine(&engine);

    let before = EngineStamp::of_tree(&engine).unwrap();
    before.write(&engine).unwrap();

    assert!(engine.join(STAMP_FILE).is_file());
    assert_eq!(before, EngineStamp::of_tree(&engine).unwrap());
}

/// The bug itself: same version, different source, and the old check
/// (`is_engine_source`) says up to date because three entries exist.
#[test]
fn a_different_source_at_the_same_version_replaces_the_engine() {
    let dir = tmp("replaces");
    let (old, new, home) = (dir.join("old_src"), dir.join("new_src"), dir.join("home"));
    fake_engine(&old);
    fake_engine(&new);
    // One file differs. That is all an editor update may amount to, and
    // it has to be enough.
    fs::write(new.join("crates/kooch_core/src/lib.rs"), "// core, fixed").unwrap();
    let dest = home.join("0.1.0/engine");

    let (first, _) = ensure_current_in(&dest, Some(&old)).unwrap();
    let (second, _) = ensure_current_in(&dest, Some(&new)).unwrap();

    assert_eq!(first, VendorState::Materialised);
    assert_eq!(
        second,
        VendorState::Replaced,
        "a newer engine did not replace the old one — this is #761",
    );
    assert_eq!(
        fs::read_to_string(dest.join("crates/kooch_core/src/lib.rs")).unwrap(),
        "// core, fixed",
        "the directory kept the stale source",
    );
}

/// The same source twice must not copy twice, or opening a project
/// re-copies 8 MB every time and an engine someone is hacking on gets
/// stomped.
#[test]
fn the_same_source_twice_is_up_to_date() {
    let dir = tmp("idempotent");
    let (engine, home) = (dir.join("engine_src"), dir.join("home"));
    fake_engine(&engine);
    let dest = home.join("0.1.0/engine");

    ensure_current_in(&dest, Some(&engine)).unwrap();
    let (state, _) = ensure_current_in(&dest, Some(&engine)).unwrap();

    assert_eq!(state, VendorState::UpToDate);
}

/// Materialised by an editor from before this existed: it has the shape
/// and no identity. Treated as stale rather than trusted, because the
/// one thing known about it is that nobody can say what it is.
#[test]
fn an_engine_with_no_stamp_is_replaced() {
    let dir = tmp("no_stamp");
    let (engine, home) = (dir.join("engine_src"), dir.join("home"));
    fake_engine(&engine);
    let dest = home.join("0.1.0/engine");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fake_engine(&dest);
    assert!(is_engine_source(&dest));

    let (state, _) = ensure_current_in(&dest, Some(&engine)).unwrap();

    assert_eq!(state, VendorState::Replaced);
    assert!(dest.join(STAMP_FILE).is_file());
}

/// 🔴 The point of the whole feature is that the same source is not
/// stored twice. A replacement that leaked its scratch directories would
/// leave three copies of the engine on disk.
#[test]
fn replacing_leaves_exactly_one_copy() {
    let dir = tmp("one_copy");
    let (old, new, home) = (dir.join("old_src"), dir.join("new_src"), dir.join("home"));
    fake_engine(&old);
    fake_engine(&new);
    fs::write(new.join("src/lib.rs"), "// facade, newer").unwrap();
    let dest = home.join("0.1.0/engine");

    ensure_current_in(&dest, Some(&old)).unwrap();
    ensure_current_in(&dest, Some(&new)).unwrap();

    assert!(is_engine_source(&dest));
    assert!(
        !dest.with_extension("partial").exists(),
        "the staging directory survived",
    );
    assert!(
        !dest.with_extension("stale").exists(),
        "the replaced engine was left on disk",
    );
}

/// A run that died between moving the old copy aside and moving the new
/// one in leaves both scratch directories behind. The next run has to
/// repair that, not fail on it — `rename` refuses a target that exists.
#[test]
fn leftovers_from_an_interrupted_swap_are_repaired() {
    let dir = tmp("interrupted");
    let (engine, home) = (dir.join("engine_src"), dir.join("home"));
    fake_engine(&engine);
    let dest = home.join("0.1.0/engine");

    let staging = dest.with_extension("partial");
    let stale = dest.with_extension("stale");
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("junk"), "half a copy").unwrap();
    fs::create_dir_all(&stale).unwrap();
    fs::write(stale.join("junk"), "yesterday's engine").unwrap();

    let (state, _) = ensure_current_in(&dest, Some(&engine)).unwrap();

    assert_eq!(state, VendorState::Materialised);
    assert!(is_engine_source(&dest));
    assert!(
        !dest.join("junk").exists(),
        "the interrupted copy leaked in"
    );
    assert!(!staging.exists());
    assert!(!stale.exists());
}

/// 🔴 A version this editor does not ship is not stale, it is somebody
/// else's. Comparing it against the source in hand would find a
/// difference every time and overwrite the engine a pinned project
/// builds against.
#[test]
fn another_version_is_never_replaced() {
    let _env = super::ENGINE_HOME_LOCK.lock().expect("env lock");
    let dir = tmp("other_version");
    let (engine, home) = (dir.join("engine_src"), dir.join("home"));
    fake_engine(&engine);
    let older = home.join("0.0.9").join(VENDOR_DIR);
    fs::create_dir_all(older.parent().unwrap()).unwrap();
    fake_engine(&older);
    fs::write(older.join("src/lib.rs"), "// the engine 0.0.9 shipped").unwrap();
    // SAFETY: single-threaded suite; see the sibling module's env tests.
    unsafe { std::env::set_var("KOOCH_ENGINE_HOME", &home) };

    let (state, path) = ensure_current("0.0.9", Some(&engine)).unwrap();

    assert_eq!(state, VendorState::UpToDate);
    assert_eq!(path.as_deref(), Some(older.as_path()));
    assert_eq!(
        fs::read_to_string(older.join("src/lib.rs")).unwrap(),
        "// the engine 0.0.9 shipped",
        "a pinned project's engine was overwritten with this editor's",
    );

    unsafe { std::env::remove_var("KOOCH_ENGINE_HOME") };
}

/// Moving a file changes the tree even when every byte of content is
/// still there — a rename is exactly the kind of engine change that
/// breaks a build.
#[test]
fn moving_a_file_changes_the_stamp() {
    let dir = tmp("moved");
    let (before, after) = (dir.join("before"), dir.join("after"));
    fake_engine(&before);
    fake_engine(&after);
    fs::rename(
        after.join("crates/kooch_core/src/lib.rs"),
        after.join("crates/kooch_core/src/core.rs"),
    )
    .unwrap();

    assert_ne!(
        EngineStamp::of_tree(&before).unwrap(),
        EngineStamp::of_tree(&after).unwrap(),
    );
}

/// 🔴 The blind spot the stamp comparison has by construction: deleting
/// a file from a copy does not change what the copy *claims* to be, so
/// nothing else on this path can see it.
#[test]
fn a_missing_file_is_found_by_checking_the_tree() {
    let dir = tmp("damaged");
    let (engine, home) = (dir.join("engine_src"), dir.join("home"));
    fake_engine(&engine);
    let dest = home.join("0.1.0/engine");
    ensure_current_in(&dest, Some(&engine)).unwrap();
    assert_eq!(EngineStamp::check(&dest).unwrap(), Check::Match);

    fs::remove_file(dest.join("crates/kooch_core/src/lib.rs")).unwrap();

    assert!(
        matches!(EngineStamp::check(&dest).unwrap(), Check::Differs { .. }),
        "a deleted source file left the engine reporting itself intact",
    );
    // And the cheap check still says up to date, which is exactly why
    // the expensive one exists.
    assert_eq!(
        EngineStamp::read(&dest),
        Some(EngineStamp::of_source(&engine).unwrap()),
    );
}

/// The other half: finding the damage is only useful if something acts
/// on it. `KOOCH_VERIFY_ENGINE` turns the check on and a mismatch
/// re-copies, which is the repair.
#[test]
fn verifying_repairs_a_damaged_engine() {
    let dir = tmp("repairs");
    let (engine, home) = (dir.join("engine_src"), dir.join("home"));
    fake_engine(&engine);
    let dest = home.join("0.1.0/engine");
    ensure_current_in(&dest, Some(&engine)).unwrap();
    let gone = dest.join("crates/kooch_core/src/lib.rs");
    fs::remove_file(&gone).unwrap();

    // Off by default — it reads the whole tree on a path that runs every
    // time a project opens.
    let (state, _) = ensure_current_in(&dest, Some(&engine)).unwrap();
    assert_eq!(state, VendorState::UpToDate);
    assert!(
        !gone.exists(),
        "the damage was repaired without being asked"
    );

    // SAFETY: single-threaded suite; see the sibling module's env tests.
    unsafe { std::env::set_var("KOOCH_VERIFY_ENGINE", "1") };
    let (state, _) = ensure_current_in(&dest, Some(&engine)).unwrap();
    unsafe { std::env::remove_var("KOOCH_VERIFY_ENGINE") };

    assert_eq!(state, VendorState::Replaced);
    assert!(gone.is_file(), "the missing file was not restored");
}

/// A truncated file is the disk-full case, and the one a size-blind
/// check would wave through.
#[test]
fn a_truncated_file_is_found_by_checking_the_tree() {
    let dir = tmp("truncated");
    let (engine, home) = (dir.join("engine_src"), dir.join("home"));
    fake_engine(&engine);
    let dest = home.join("0.1.0/engine");
    ensure_current_in(&dest, Some(&engine)).unwrap();

    fs::write(dest.join("src/lib.rs"), "").unwrap();

    assert!(matches!(
        EngineStamp::check(&dest).unwrap(),
        Check::Differs { .. }
    ));
}

/// Nothing recorded is not damage — it is the stale case, which the
/// cheap comparison already replaces.
#[test]
fn a_tree_with_no_stamp_reports_no_stamp() {
    let dir = tmp("check_unstamped");
    let engine = dir.join("engine_src");
    fake_engine(&engine);

    assert_eq!(EngineStamp::check(&engine).unwrap(), Check::NoStamp);
}

/// Recomputing on every open would read 8 MB each time a project is
/// opened. A source that already carries a stamp is believed.
#[test]
fn a_source_that_carries_a_stamp_propagates_it() {
    let dir = tmp("propagates");
    let engine = dir.join("engine_src");
    fake_engine(&engine);
    let declared = EngineStamp {
        engine_version: "9.9.9-packaged".to_owned(),
        tree_hash: 0xdead_beef,
    };
    declared.write(&engine).unwrap();

    assert_eq!(EngineStamp::of_source(&engine).unwrap(), declared);
}
