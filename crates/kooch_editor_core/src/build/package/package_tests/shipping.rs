//! What ships and what stays behind: presets, sidecars, settings, engine assets, the manifest.

use super::*;

/// 🔴 A `.buildpreset` describes how to *make* the game. The game never
/// reads one, and shipping it hands anyone who opens the pack a
/// description of how it is built — output folder, target, features.
#[test]
fn build_presets_do_not_ship() {
    let dir = tmp("nopreset");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    write(
        &proj.join("assets/LinuxBuild.buildpreset"),
        b"(output_dir: \"build\")",
    );
    write(
        &proj.join("assets/LinuxBuild.buildpreset.meta"),
        b"guid = \"x\"",
    );

    let out = assemble(
        &BuildPreset::default(),
        Platform::Linux,
        &known(),
        &proj,
        Some(&eng),
        &binary(&dir),
        "demo",
        &key,
    )
    .unwrap();

    let pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    let shipped: Vec<&str> = pack.entries().iter().map(|e| e.name.as_str()).collect();
    assert!(
        !shipped.iter().any(|name| name.contains("buildpreset")),
        "a build preset shipped inside the game: {shipped:?}",
    );
    // And the game's own content still did.
    assert!(shipped.contains(&"assets/props/rock.glb"));
}

/// ⚠️ The sidecar goes with what it describes. Left behind it would be an
/// orphan the pack scan counts and nothing resolves.
#[test]
fn an_authoring_sidecar_does_not_ship_either() {
    let dir = tmp("nopresetmeta");
    let key = PackKey::generate();
    let proj = dir.join("proj");
    project(&proj);
    write(&proj.join("assets/A.buildpreset"), b"()");
    write(&proj.join("assets/A.buildpreset.meta"), b"guid = \"x\"");

    let out = assemble(
        &BuildPreset::default(),
        Platform::Linux,
        &known(),
        &proj,
        None,
        &binary(&dir),
        "demo",
        &key,
    )
    .unwrap();

    let pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert!(
        !pack
            .entries()
            .iter()
            .any(|e| e.name.contains("buildpreset"))
    );
}

/// 🔴 The opposite case, and the one it would be easy to break by widening the filter:
/// `.rendersettings` is what the project *looks* like and the renderer reads it at startup.
#[test]
fn render_settings_still_ship() {
    let dir = tmp("settingsship");
    let key = PackKey::generate();
    let proj = dir.join("proj");
    project(&proj);
    write(&proj.join("assets/project.rendersettings"), b"()");

    let out = assemble(
        &BuildPreset::default(),
        Platform::Linux,
        &known(),
        &proj,
        None,
        &binary(&dir),
        "demo",
        &key,
    )
    .unwrap();

    let pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert!(
        pack.contains("assets/project.rendersettings"),
        "the project's look did not ship",
    );
}

/// 🔴 The bug this replaced a curated list to fix: a scene using the engine's `suzanne.glb` shipped
/// without it and rendered nothing.
#[test]
fn an_engine_asset_the_scene_uses_ships() {
    let dir = tmp("suzanne");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    // Outside `meshes/primitives`, which is exactly where the old list stopped looking.
    write(&eng.join("assets/meshes/suzanne.glb"), b"suzanne");
    write(
        &eng.join("assets/meshes/suzanne.glb.meta"),
        b"guid = \"0b1ec7a0-0000-4000-8000-000000000001\"\n",
    );
    // A scene that names it, the way a real one does.
    write(
        &proj.join(kooch_core::scene_paths::DEFAULT_SCENE_REL_PATH),
        br#"(entities: [(components: [(fields: [("mesh", AssetRef(
            guid: Some("0b1ec7a0-0000-4000-8000-000000000001"),
        ))])])])"#,
    );

    let out = assemble(
        &BuildPreset::default(),
        Platform::Linux,
        &known(),
        &proj,
        Some(&eng),
        &binary(&dir),
        "demo",
        &key,
    )
    .unwrap();

    let mut pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert_eq!(
        pack.read("assets/meshes/suzanne.glb").unwrap(),
        b"suzanne",
        "the mesh the scene draws did not ship",
    );
}

/// And the other half: 12 of the engine's 13 MB are demo models no game
/// loads, so what nothing names stays behind.
#[test]
fn an_engine_asset_nothing_uses_stays_behind() {
    let dir = tmp("unused");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    write(&eng.join("assets/meshes/demo.glb"), b"12 MB of demo");
    write(
        &eng.join("assets/meshes/demo.glb.meta"),
        b"guid = \"deadbeef-0000-4000-8000-000000000001\"\n",
    );
    write(
        &proj.join(kooch_core::scene_paths::DEFAULT_SCENE_REL_PATH),
        b"(entities: [])",
    );

    let out = assemble(
        &BuildPreset::default(),
        Platform::Linux,
        &known(),
        &proj,
        Some(&eng),
        &binary(&dir),
        "demo",
        &key,
    )
    .unwrap();

    let pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert!(
        !pack.contains("assets/meshes/demo.glb"),
        "a demo model shipped"
    );
}

/// 🔴 Without this the game cannot know which scene it opens with, and `main_scene` goes back to
/// being a field nothing reads (#808).
#[test]
fn the_manifest_travels_beside_the_binary() {
    let dir = tmp("manifest");
    // `run` packages `<dir>/proj`, so the manifest goes where the project
    // is rather than where the fixture starts.
    write(
        &dir.join("proj")
            .join(kooch_core::scene_paths::PROJECT_MANIFEST_FILE),
        br#"(name: "demo", main_scene: Some("assets/scenes/level.scene"))"#,
    );
    let out = run(&dir, &BuildPreset::default()).unwrap();

    let shipped = out.dir.join(kooch_core::scene_paths::PROJECT_MANIFEST_FILE);
    assert!(
        shipped.is_file(),
        "the game has no manifest to read its starting scene from",
    );
    assert_eq!(
        kooch_core::scene_paths::main_scene_of(&std::fs::read_to_string(&shipped).unwrap())
            .as_deref(),
        Some("assets/scenes/level.scene"),
    );
}

/// A project built before #808 has no manifest to copy, and that is not
/// an error: the convention path is what such a build has always used.
#[test]
fn a_project_without_a_manifest_still_packages() {
    let dir = tmp("no_manifest");
    let out = run(&dir, &BuildPreset::default()).unwrap();

    assert!(out.binary.is_file());
    assert!(
        !out.dir
            .join(kooch_core::scene_paths::PROJECT_MANIFEST_FILE)
            .exists(),
    );
}
