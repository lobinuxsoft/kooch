use super::*;

/// A minimal directory that passes `is_engine_source`.
fn fake_engine(root: &Path) {
    fs::create_dir_all(root.join("crates/kooch_ecs/src")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]").unwrap();
    fs::write(root.join("src/lib.rs"), "// facade").unwrap();
    fs::write(root.join("crates/kooch_ecs/src/lib.rs"), "// ecs").unwrap();
    fs::write(root.join("LICENSE.md"), "# All Rights Reserved").unwrap();
}

/// 🔴 The whole of #754, as one assertion.
///
/// The manifest must not carry a path that exists only on the
/// machine that generated it, and the engine must **not** be copied
/// into the project — one per machine, shared by every project.
#[test]
fn a_generated_project_points_at_the_shared_engine_and_contains_none() {
    let _env = crate::engine_vendor::ENGINE_HOME_LOCK
        .lock()
        .expect("env lock");
    let tmp = std::env::temp_dir().join("kooch_project_shared_test");
    let _ = fs::remove_dir_all(&tmp);
    let engine = tmp.join("some/deep/install/kooch");
    fs::create_dir_all(&engine).unwrap();
    fake_engine(&engine);
    let parent = tmp.join("workspace");
    fs::create_dir_all(&parent).unwrap();

    // SAFETY: single-threaded test; nothing else reads the
    // environment concurrently. Keeps this out of the real
    // ~/.local/share.
    let home = tmp.join("engine_home");
    unsafe { std::env::set_var("KOOCH_ENGINE_HOME", &home) };

    let project = create_project("my game", &parent, &engine).expect("creates");
    let manifest = fs::read_to_string(project.join("Cargo.toml")).unwrap();

    assert!(
        !project.join("engine").exists(),
        "the engine was copied into the project; it is meant to be shared",
    );
    assert!(
        !manifest.contains(&engine.display().to_string()),
        "the creating machine's engine path leaked into the manifest:\n{manifest}",
    );
    assert!(
        manifest.contains(&home.display().to_string()),
        "the manifest should point at the shared engine:\n{manifest}",
    );
    assert!(
        home.join(format!(
            "{}/engine/crates/kooch_ecs/src/lib.rs",
            crate::engine_vendor::editor_engine_version()
        ))
        .is_file(),
        "the shared engine was referenced but never materialised",
    );

    unsafe { std::env::remove_var("KOOCH_ENGINE_HOME") };
}

/// `$HOME` differs per user, so a project that changed machines
/// names a directory that is not there. The editor owns that line
/// and corrects it on open rather than letting cargo fail on it.
#[test]
fn opening_a_project_repoints_a_stale_engine_path() {
    let _env = crate::engine_vendor::ENGINE_HOME_LOCK
        .lock()
        .expect("env lock");
    let tmp = std::env::temp_dir().join("kooch_repoint_test");
    let _ = fs::remove_dir_all(&tmp);
    let project = tmp.join("proj");
    fs::create_dir_all(&project).unwrap();
    fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"g\"\n\n[dependencies]\n\
             kooch = { path = \"/home/someone-else/.local/share/kooch/0.1.0/engine\", \
             features = [\"editor\"] }\n\
             kooch_ecs = { path = \"/home/someone-else/.local/share/kooch/0.1.0/engine/crates/kooch_ecs\" }\n",
        )
        .unwrap();

    let here = tmp.join("mine/0.1.0/engine");
    let changed = point_manifest_at_engine(&project, &here).expect("rewrites");

    assert!(changed);
    let manifest = fs::read_to_string(project.join("Cargo.toml")).unwrap();
    assert!(!manifest.contains("someone-else"), "{manifest}");
    assert!(
        manifest.contains(&format!("path = \"{}\"", here.display())),
        "{manifest}"
    );
    assert!(
        manifest.contains(&format!("{}/crates/kooch_ecs", here.display())),
        "the second dependency was left pointing at the old machine:\n{manifest}",
    );
    // Features and everything else on the line survive.
    assert!(manifest.contains("features = [\"editor\"]"), "{manifest}");

    // Idempotent: opening again must not rewrite for nothing.
    assert!(!point_manifest_at_engine(&project, &here).expect("second pass"));
}

/// 🔴 Two files record which engine a project uses, and moving it has to
/// write both.
///
/// `Cargo.toml` carries the **path** — that is what cargo compiles
/// against, and `point_manifest_at_engine` rewrites it. `project.kooch`
/// carries `engine_version`, and that is what `ProjectState::open` hands
/// to `engine_vendor::status`.
///
/// Updating only the first leaves a project pointing at 0.2.0 on disk
/// while still calling itself 0.1.0, so the engine prompt returns on
/// every open no matter how many times Install is pressed — the bug this
/// test exists for. Nothing fails; it just never stops asking.
#[test]
fn moving_a_project_takes_two_writes() {
    let tmp = std::env::temp_dir().join("kooch_project_two_records");
    let _ = fs::remove_dir_all(&tmp);
    let (project, new_engine) = (tmp.join("game"), tmp.join("share/0.2.0/engine"));
    fs::create_dir_all(&project).unwrap();
    fake_engine(&new_engine);

    fs::write(
        project.join("Cargo.toml"),
        "[dependencies]\nkooch = { path = \"/old/share/0.1.0/engine\" }\n",
    )
    .unwrap();
    let mut manifest = ProjectManifest::new("game");
    manifest.engine_version = "0.1.0".to_owned();
    manifest.save(&project).unwrap();

    // What Install did, and only this.
    point_manifest_at_engine(&project, &new_engine).unwrap();

    let cargo = fs::read_to_string(project.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("0.2.0"), "cargo manifest was not repointed");
    assert_eq!(
        ProjectManifest::load(&project).unwrap().engine_version,
        "0.1.0",
        "this is the trap: the cargo path moved and the recorded version did not",
    );

    // The other half, which `update_engine` now does.
    manifest.engine_version = "0.2.0".to_owned();
    manifest.save(&project).unwrap();
    assert_eq!(
        ProjectManifest::load(&project).unwrap().engine_version,
        "0.2.0",
        "the project still does not agree with what it builds against",
    );
}
