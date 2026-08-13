use super::copy::is_test_code;
use super::*;

/// Builds a directory that passes for an engine root, with a
/// `target/` in it to be skipped.
fn fake_engine(root: &Path) {
    fs::create_dir_all(root.join("crates/kooch_core/src")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("assets/materials")).unwrap();
    fs::create_dir_all(root.join("assets/meshes/primitives")).unwrap();
    fs::create_dir_all(root.join("target/debug")).unwrap();
    fs::create_dir_all(root.join("crates/kooch_core/target")).unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]").unwrap();
    fs::write(root.join("Cargo.lock"), "# lock").unwrap();
    fs::write(root.join("src/lib.rs"), "// facade").unwrap();
    fs::write(root.join("crates/kooch_core/src/lib.rs"), "// core").unwrap();
    fs::write(root.join("assets/materials/default.material"), "()").unwrap();
    fs::write(root.join("assets/meshes/primitives/cube.glb"), "glb").unwrap();
    fs::write(root.join("assets/meshes/demo.glb"), &vec![0u8; 4096]).unwrap();
    fs::write(root.join("target/debug/huge"), &vec![0u8; 65536]).unwrap();
    fs::write(
        root.join("crates/kooch_core/target/nested"),
        &vec![0u8; 65536],
    )
    .unwrap();
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_vendor_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_vendored_engine_has_what_a_build_needs() {
    let dir = tmp("copies");
    let (engine, project) = (dir.join("engine_src"), dir.join("proj"));
    fake_engine(&engine);
    fs::create_dir_all(&project).unwrap();

    let dest = vendor_engine(&project, &engine).expect("vendors");

    assert!(dest.join("Cargo.toml").is_file());
    assert!(dest.join("Cargo.lock").is_file());
    assert!(dest.join("src/lib.rs").is_file());
    assert!(dest.join("crates/kooch_core/src/lib.rs").is_file());
}

/// 🔴 The check that keeps this feature from being a disaster. A
/// missed `target/` turns "8 MB of source" into a copy of somebody's
/// entire build directory, and the nested one is the easy miss: a
/// workspace member built standalone has its own.
#[test]
fn no_build_output_is_copied_at_any_depth() {
    let dir = tmp("skips_target");
    let (engine, project) = (dir.join("engine_src"), dir.join("proj"));
    fake_engine(&engine);
    fs::create_dir_all(&project).unwrap();

    let dest = vendor_engine(&project, &engine).expect("vendors");

    assert!(!dest.join("target").exists(), "top-level target/ copied");
    assert!(
        !dest.join("crates/kooch_core/target").exists(),
        "a nested target/ was copied — the recursion only checks the top",
    );
}

/// The engine's `assets/` is 13 MB and 12.7 MB of it is demo models.
/// A project gets what it runs on, not the samples.
#[test]
fn assets_are_the_ones_a_game_runs_on_not_the_demos() {
    let dir = tmp("assets");
    let (engine, project) = (dir.join("engine_src"), dir.join("proj"));
    fake_engine(&engine);
    fs::create_dir_all(&project).unwrap();

    let dest = vendor_engine(&project, &engine).expect("vendors");

    assert!(dest.join("assets/materials/default.material").is_file());
    assert!(dest.join("assets/meshes/primitives/cube.glb").is_file());
    assert!(
        !dest.join("assets/meshes/demo.glb").exists(),
        "a demo model was vendored into every project",
    );
}

/// The distinction the whole feature turns on. An installed editor
/// that ships source next to itself must still vendor, or nobody
/// ever gets a self-contained project.
#[test]
fn shipping_source_next_to_the_editor_is_not_developing_the_engine() {
    let dir = tmp("dev_detection");
    let installed = dir.join("opt_kooch");
    fake_engine(&installed);
    assert!(
        is_engine_source(&installed),
        "the fixture should look like engine source — that is the point",
    );
    assert!(
        !running_from_engine_build(&installed),
        "the test binary does not live under this fixture's target/, \
             so this must read as an installed editor",
    );
}

/// First run on a machine, or first project after an editor
/// update: nothing is there and it has to appear.
#[test]
fn a_missing_engine_is_materialised() {
    let dir = tmp("materialise");
    let (engine, home) = (dir.join("editor_src"), dir.join("home"));
    fake_engine(&engine);

    let (state, path) = ensure_current_in(&home.join("0.1.0/engine"), Some(&engine)).unwrap();

    assert_eq!(state, VendorState::Materialised);
    let path = path.expect("a materialised engine has a path");
    assert!(path.join("crates/kooch_core/src/lib.rs").is_file());
}

/// 🔴 Two versions coexist. A project pinned to an older engine has
/// to keep building after the editor updates, so materialising a
/// new one must not disturb the old.
#[test]
fn versions_live_side_by_side() {
    let dir = tmp("versions");
    let (engine, home) = (dir.join("editor_src"), dir.join("home"));
    fake_engine(&engine);

    ensure_current_in(&home.join("0.1.0/engine"), Some(&engine)).unwrap();
    ensure_current_in(&home.join("0.2.0/engine"), Some(&engine)).unwrap();

    assert!(home.join("0.1.0/engine/Cargo.toml").is_file());
    assert!(home.join("0.2.0/engine/Cargo.toml").is_file());
}

/// Re-copying on every open would make opening a project feel
/// broken, and would stomp an engine someone is deliberately
/// hacking on.
#[test]
fn an_existing_engine_is_left_alone() {
    let dir = tmp("uptodate");
    let (engine, home) = (dir.join("editor_src"), dir.join("home"));
    fake_engine(&engine);
    let dest = home.join("0.1.0/engine");
    ensure_current_in(&dest, Some(&engine)).unwrap();

    let marker = dest.join("src/local_edit.rs");
    fs::write(&marker, "// mine").unwrap();

    let (state, _) = ensure_current_in(&dest, Some(&engine)).unwrap();

    assert_eq!(state, VendorState::UpToDate);
    assert!(marker.is_file(), "an existing engine was re-copied");
}

/// 🔴 An interrupted copy must not leave a half-written directory
/// that passes for an engine — it would never be repaired, and
/// every project on the machine would fail to build against it.
/// The copy stages beside the destination and is renamed in.
#[test]
fn a_materialised_engine_appears_atomically() {
    let dir = tmp("atomic");
    let (engine, home) = (dir.join("editor_src"), dir.join("home"));
    fake_engine(&engine);
    let dest = home.join("0.1.0/engine");

    // A leftover from a previous interrupted run.
    let staging = dest.with_extension("partial");
    fs::create_dir_all(staging.join("crates")).unwrap();
    fs::write(staging.join("junk"), "half a copy").unwrap();

    ensure_current_in(&dest, Some(&engine)).unwrap();

    assert!(is_engine_source(&dest));
    assert!(!staging.exists(), "the staging directory survived");
    assert!(
        !dest.join("junk").exists(),
        "the interrupted copy leaked in"
    );
}

/// No source and nothing there: say so rather than inventing a
/// path a manifest would then point at.
#[test]
fn without_source_there_is_no_engine_and_no_path() {
    let dir = tmp("nosource");
    let home = dir.join("home");

    let (state, path) = ensure_current_in(&home.join("0.1.0/engine"), None).unwrap();

    assert_eq!(state, VendorState::NoSourceAvailable);
    assert_eq!(path, None);
}

/// 🔴 The rule this whole arrangement serves: **no test code leaves
/// the engine repo.** Vendors the real engine tree — not a fixture —
/// and asserts nothing test-shaped survives.
///
/// A fixture would only prove the filter matches what the fixture
/// contains. The engine has 237 test modules and a `tests/` directory
/// per crate; this reads those.
#[test]
fn the_vendored_engine_contains_no_test_code() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels under the repo root")
        .to_path_buf();
    let dir = tmp("no_tests");
    fs::create_dir_all(&dir).unwrap();

    let dest = vendor_engine(&dir, &repo).expect("vendors the real engine");

    let mut offenders: Vec<String> = Vec::new();
    let mut stack = vec![dest.clone()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if is_test_code(&path, &[]) {
                    offenders.push(format!("{} (directory)", path.display()));
                }
                stack.push(path);
                continue;
            }
            if is_test_code(&path, &[]) {
                offenders.push(path.display().to_string());
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap_or_default();
            // `mod tests;` is a declaration, and `#[cfg(test)]` removes
            // it before Rust resolves the module — it compiles to
            // nothing. A `#[test]` or an inline `mod tests {` is the
            // real thing.
            if text.contains("#[test]") || text.contains("mod tests {") {
                offenders.push(format!("{} (contains test code)", path.display()));
            }
        }
    }

    let _ = fs::remove_dir_all(&dir);
    assert!(
        offenders.is_empty(),
        "test code was vendored into a project:\n  {}",
        offenders.join("\n  "),
    );
}

/// 🔴 The licence is not optional and not a courtesy copy: the
/// facade compiles it in with `include_str!`, so a materialised
/// engine missing it fails to build. This asserts the vendor
/// carries it by name, and `reach_tests` independently asserts that
/// anything the source `include_str!`s is vendored — two different
/// reasons for the same file to be there.
#[test]
fn the_licence_travels_with_the_engine() {
    let dir = tmp("licence");
    let (engine, project) = (dir.join("engine_src"), dir.join("proj"));
    fake_engine(&engine);
    fs::write(engine.join("LICENSE.md"), "# All Rights Reserved").unwrap();
    fs::create_dir_all(&project).unwrap();

    let dest = vendor_engine(&project, &engine).expect("vendors");

    let licence = dest.join("LICENSE.md");
    assert!(
        licence.is_file(),
        "the engine was vendored without its licence"
    );
    assert!(
        fs::read_to_string(&licence)
            .unwrap()
            .contains("All Rights Reserved"),
        "the vendored licence is not the licence",
    );
}

/// Failing here is cheap; failing at `cargo build` in a project
/// whose engine directory is half a copy is not, because nothing in
/// that error mentions vendoring.
#[test]
fn a_directory_that_is_not_the_engine_is_refused_before_writing() {
    let dir = tmp("refuses");
    let (not_engine, project) = (dir.join("somewhere"), dir.join("proj"));
    fs::create_dir_all(&not_engine).unwrap();
    fs::create_dir_all(&project).unwrap();

    assert!(matches!(
        vendor_engine(&project, &not_engine),
        Err(VendorError::NotAnEngineRoot(_)),
    ));
    assert!(!project.join(VENDOR_DIR).exists(), "wrote before checking");
}

/// 🔴 A test that writes into the developer's real `~/.local/share` is
/// not a test, it is a side effect — and this one is worse than untidy:
/// `is_engine_source` accepts what it leaves behind, so the editor
/// reports the engine up to date and never materialises the real one.
///
/// Without the override, `shared_engine_dir` refuses to answer under
/// `cfg(test)` rather than handing back a real path.
#[test]
fn tests_cannot_reach_the_real_data_directory() {
    let _env = super::ENGINE_HOME_LOCK.lock().expect("env lock");
    // SAFETY: single-threaded by `--test-threads=1` in the suite that
    // needs it; nothing else reads the environment here.
    unsafe { std::env::remove_var("KOOCH_ENGINE_HOME") };
    assert_eq!(
        shared_engine_dir("0.1.0"),
        None,
        "a test just resolved the real shared engine directory",
    );

    let home = tmp("override");
    unsafe { std::env::set_var("KOOCH_ENGINE_HOME", &home) };
    assert_eq!(
        shared_engine_dir("0.1.0"),
        Some(home.join("0.1.0").join(VENDOR_DIR)),
    );
    unsafe { std::env::remove_var("KOOCH_ENGINE_HOME") };
}

/// 🔴 An editor may only materialise the engine it ships. Asked for a
/// version it does not have, it must NOT write its own source into a
/// directory named after the other one — that puts an engine on disk
/// under a name that is not its own, and everything downstream trusts
/// the name.
#[test]
fn an_editor_never_materialises_a_version_it_does_not_have() {
    let _env = super::ENGINE_HOME_LOCK.lock().expect("env lock");
    let dir = tmp("wrong_version");
    let (engine, home) = (dir.join("editor_src"), dir.join("home"));
    fake_engine(&engine);
    // SAFETY: single-threaded suite; see the module's other env tests.
    unsafe { std::env::set_var("KOOCH_ENGINE_HOME", &home) };

    let (state, path) = ensure_current("0.0.1-from-another-editor", Some(&engine)).unwrap();

    assert_eq!(state, VendorState::Materialised);
    let path = path.expect("something was materialised");
    assert!(
        path.to_string_lossy().contains(editor_engine_version()),
        "materialised into {}, which is not this editor's version",
        path.display(),
    );
    assert!(
        !home.join("0.0.1-from-another-editor").exists(),
        "wrote this editor's engine under another version's name",
    );

    unsafe { std::env::remove_var("KOOCH_ENGINE_HOME") };
}

/// The other half: a version the machine DOES have is honoured, so a
/// project pinned to an older engine keeps building after the editor
/// updates.
#[test]
fn a_version_already_on_the_machine_is_honoured() {
    // 🔴 The same lock the two tests above take. `KOOCH_ENGINE_HOME` is
    // process-wide, and cargo's test harness is not single-threaded: one
    // test removing the variable while this one reads it makes
    // `ensure_current` resolve against a directory that is not there, and
    // the failure is `Io(NotFound)` from a line that never touches the
    // filesystem itself. Intermittent, and it only shows up in the full
    // suite — it passes alone and under `--test-threads=1`.
    //
    // Same bug as `KOOCH_PACK_KEY`, whose fix took the lock in all seven
    // tests of its module including the ones that never set the variable.
    // This module stopped at two of seven.
    let _env = super::ENGINE_HOME_LOCK.lock().expect("env lock");
    let dir = tmp("honour_existing");
    let (engine, home) = (dir.join("editor_src"), dir.join("home"));
    fake_engine(&engine);
    let older = home.join("0.0.9").join(VENDOR_DIR);
    fs::create_dir_all(older.parent().unwrap()).unwrap();
    fake_engine(&older);
    unsafe { std::env::set_var("KOOCH_ENGINE_HOME", &home) };

    let (state, path) = ensure_current("0.0.9", Some(&engine)).unwrap();

    assert_eq!(state, VendorState::UpToDate);
    assert_eq!(path.as_deref(), Some(older.as_path()));

    unsafe { std::env::remove_var("KOOCH_ENGINE_HOME") };
}

/// 🔴 Install has to move the project onto **this editor's** version.
///
/// `ensure_current` honours a project's own version when that engine is
/// already on the machine (`engine_vendor.rs:314`) — deliberately, so a
/// project pinned to an older engine keeps building. But Install asked
/// for exactly that version, so with 0.1.0 already on disk and the
/// editor shipping 0.2.0 the call returned `UpToDate` pointing at the
/// *old* directory: the button installed nothing, reported nothing, and
/// the prompt came straight back. Meanwhile it promises "Installing
/// moves the project onto it".
///
/// Verified failing: ask with `project_version` and the returned path is
/// the 0.1.0 directory that was already there.
#[test]
fn install_moves_the_project_to_the_editors_version() {
    let _env = super::ENGINE_HOME_LOCK.lock().expect("env lock");
    let dir = tmp("install_other_version");
    let (engine, home) = (dir.join("editor_src"), dir.join("home"));
    fake_engine(&engine);
    // The older engine the project is pinned to, already on the machine.
    let old = home.join("0.0.1").join(VENDOR_DIR);
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fake_engine(&old);
    unsafe { std::env::set_var("KOOCH_ENGINE_HOME", &home) };

    let status = EngineStatus {
        project_version: "0.0.1".to_owned(),
        editor_version: super::editor_engine_version().to_owned(),
        installed: Some(old.clone()),
        difference: Difference::OtherVersion,
    };
    assert_eq!(status.version_to_install(), super::editor_engine_version());

    let (_, path) = ensure_current(status.version_to_install(), Some(&engine))
        .expect("installing the editor's own version has to work");
    let path = path.expect("Install produced no engine — the button that does nothing");

    assert_ne!(
        path, old,
        "Install left the project on the old engine, which is the bug",
    );
    assert!(
        path.to_string_lossy()
            .contains(super::editor_engine_version()),
        "installed somewhere that is not this editor's version: {}",
        path.display(),
    );

    unsafe { std::env::remove_var("KOOCH_ENGINE_HOME") };
}
