//! #758 — what a player receives, and what must never be deleted to
//! produce it.

use super::*;

use kooch_pack::Pack;

/// The extensions a loader claims, as the real allowlist would hand
/// them over. The fixtures use these, so the tests exercise the filter
/// rather than bypassing it.
fn known() -> Vec<String> {
    [
        "glb",
        "ron",
        "prefab",
        "rendersettings",
        "buildpreset",
        "scene",
    ]
    .iter()
    .map(|e| (*e).to_owned())
    .collect()
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_pkg_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// Guids the fixtures use, so a scene can name an engine asset the way a
/// real one does.
const ENGINE_MATERIAL: &str = "11111111-0000-4000-8000-000000000001";
const ENGINE_CUBE: &str = "22222222-0000-4000-8000-000000000002";

/// A project with a scene, an asset and its sidecar.
///
/// The scene names the engine assets it draws — which is what decides
/// whether they travel, since a curated list guessed wrong once already.
fn project(root: &Path) {
    write(
        &root.join(kooch_core::scene_paths::DEFAULT_SCENE_REL_PATH),
        format!(
            r#"(entities: [(components: [(fields: [
                ("material", AssetRef(guid: Some("{ENGINE_MATERIAL}"))),
                ("mesh", AssetRef(guid: Some("{ENGINE_CUBE}"))),
            ])])])"#
        )
        .as_bytes(),
    );
    write(&root.join("assets/props/rock.glb"), b"rock mesh");
    write(&root.join("assets/props/rock.glb.meta"), b"guid = \"r\"\n");
}

/// An engine root: two assets a scene names, and a demo nothing does.
fn engine(root: &Path) {
    write(
        &root.join("assets/materials/default.ron"),
        b"engine material",
    );
    write(
        &root.join("assets/materials/default.ron.meta"),
        format!("guid = \"{ENGINE_MATERIAL}\"\n").as_bytes(),
    );
    write(&root.join("assets/meshes/primitives/cube.glb"), b"cube");
    write(
        &root.join("assets/meshes/primitives/cube.glb.meta"),
        format!("guid = \"{ENGINE_CUBE}\"\n").as_bytes(),
    );
    // A demo no scene names. 12 of the engine's 13 MB are these.
    write(&root.join("assets/meshes/demo.glb"), b"12 MB of demo");
    write(
        &root.join("assets/meshes/demo.glb.meta"),
        b"guid = \"99999999-0000-4000-8000-000000000009\"\n",
    );
}

fn binary(dir: &Path) -> PathBuf {
    let path = dir.join("game_binary");
    write(&path, b"#!/bin/sh\necho game\n");
    path
}

fn run(dir: &Path, preset: &BuildPreset) -> Result<Package, PackageError> {
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    let exe = binary(dir);
    assemble(
        preset,
        &known(),
        &proj,
        Some(&eng),
        &exe,
        "demo",
        &PackKey::generate(),
    )
}

#[test]
fn a_package_holds_the_game_its_scenes_and_a_pack() {
    let dir = tmp("layout");
    let out = run(&dir, &BuildPreset::default()).unwrap();

    assert!(out.binary.is_file());
    assert_eq!(
        out.binary.file_name().unwrap().to_string_lossy(),
        format!("demo.{}", std::env::consts::ARCH),
    );
    assert!(out.dir.join(PACK_FILE).is_file());
    assert_eq!(out.scenes, 1);
    // 🔴 Inside the pack, not beside it. A scene is the structure of the
    // whole game, and leaving it in plain RON next to an encrypted pack
    // protects the textures and publishes the design.
    assert!(
        !out.dir
            .join(kooch_core::scene_paths::DEFAULT_SCENE_REL_PATH)
            .exists(),
        "the scene shipped in the clear",
    );
}

/// 🔴 The merge. In the editor a project has two asset roots; a shipped
/// game has one. Get this wrong and the game loads its scene and draws
/// nothing, because every engine GUID fails to resolve.
#[test]
fn both_asset_trees_land_in_one_pack() {
    let dir = tmp("merge");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    let exe = binary(&dir);

    let out = assemble(
        &BuildPreset::default(),
        &known(),
        &proj,
        Some(&eng),
        &exe,
        "demo",
        &key,
    )
    .unwrap();

    let mut pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert_eq!(
        pack.read("assets/materials/default.ron").unwrap(),
        b"engine material"
    );
    assert_eq!(
        pack.read("assets/meshes/primitives/cube.glb").unwrap(),
        b"cube"
    );
    assert_eq!(pack.read("assets/props/rock.glb").unwrap(), b"rock mesh");
}

/// ⚠️ A scene references assets by GUID and the GUID lives in the
/// sidecar. A packer that filtered by extension would produce a game that
/// loads its scene and renders nothing.
#[test]
fn meta_sidecars_travel() {
    let dir = tmp("meta");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);

    let out = assemble(
        &BuildPreset::default(),
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
        pack.entries().iter().any(|e| e.name.ends_with(".meta")),
        "no sidecar travelled — the GUIDs are gone",
    );
}

/// The engine's `assets/` is 13 MB and most of it is demo models no
/// shipped game loads.
#[test]
fn engine_demos_stay_behind() {
    let dir = tmp("demos");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);

    let out = assemble(
        &BuildPreset::default(),
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

/// The project is the author and wins — refusing the build would mean a
/// name nobody chose could stop a game from being made. But it is
/// reported, because the engine's version is simply gone.
#[test]
fn a_project_asset_shadows_the_engines() {
    let dir = tmp("shadow");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    write(&proj.join("assets/materials/default.ron"), b"mine");
    write(
        &proj.join("assets/materials/default.ron.meta"),
        format!("guid = \"{ENGINE_MATERIAL}\"\n").as_bytes(),
    );

    let out = assemble(
        &BuildPreset::default(),
        &known(),
        &proj,
        Some(&eng),
        &binary(&dir),
        "demo",
        &key,
    )
    .unwrap();

    assert_eq!(out.shadowed, vec!["assets/materials/default.ron"]);
    let mut pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert_eq!(pack.read("assets/materials/default.ron").unwrap(), b"mine");
}

/// 🔴 The output path comes from a text field in a preset, and packaging
/// empties it. `output_dir: "."` would take the project with it.
#[test]
fn packaging_refuses_to_delete_a_project() {
    let dir = tmp("guard");
    let proj = dir.join("proj");
    project(&proj);
    let exe = binary(&dir);

    // 🔴 The first version of the guard asked "does the output contain a
    // `src`?", which is true of the project root and false of `src`
    // itself. Every one of these was reachable.
    for target in [
        ".",
        "src",
        "assets",
        "scenes",
        ".kooch",
        "assets/props",
        "./assets",
        "build/../src",
    ] {
        std::fs::create_dir_all(proj.join("src")).unwrap();
        let preset = BuildPreset {
            output_dir: target.to_owned(),
            ..Default::default()
        };
        let result = assemble(
            &preset,
            &known(),
            &proj,
            None,
            &exe,
            "demo",
            &PackKey::generate(),
        );
        assert!(
            matches!(result, Err(PackageError::UnsafeOutput(_))),
            "packaging into {target:?} was allowed",
        );
    }
    // And nothing was taken with it.
    assert!(proj.join("assets/props/rock.glb").is_file());
    assert!(
        proj.join(kooch_core::scene_paths::DEFAULT_SCENE_REL_PATH)
            .is_file()
    );
}

/// A previous build's leftovers must not ship inside the next one.
#[test]
fn a_stale_output_is_cleared() {
    let dir = tmp("stale");
    let out = run(&dir, &BuildPreset::default()).unwrap();
    let leftover = out.dir.join("from_last_time.txt");
    write(&leftover, b"old");

    let out = run(&dir, &BuildPreset::default()).unwrap();

    assert!(!leftover.exists(), "a stale file survived into the build");
    assert!(out.binary.is_file());
}

/// Loose assets, for working out why a build behaves differently from
/// the editor.
#[test]
fn unpacked_assets_are_copied_as_files() {
    let dir = tmp("loose");
    let preset = BuildPreset {
        pack_assets: false,
        ..Default::default()
    };

    let out = run(&dir, &preset).unwrap();

    assert!(out.pack.is_none());
    assert!(!out.dir.join(PACK_FILE).exists());
    assert_eq!(
        std::fs::read(out.dir.join("assets/props/rock.glb")).unwrap(),
        b"rock mesh",
    );
    assert!(out.dir.join("assets/materials/default.ron").is_file());
}

#[test]
fn a_windows_preset_names_an_exe() {
    let dir = tmp("windows");
    let preset = BuildPreset {
        target_triple: "x86_64-pc-windows-gnu".to_owned(),
        ..Default::default()
    };

    let out = run(&dir, &preset).unwrap();

    assert_eq!(out.binary.file_name().unwrap(), "demo.exe");
}

/// The build ran, the binary did not appear where it was expected: say
/// that, rather than shipping a folder with no game in it.
#[test]
fn a_missing_binary_is_named() {
    let dir = tmp("nobinary");
    let proj = dir.join("proj");
    project(&proj);

    let result = assemble(
        &BuildPreset::default(),
        &known(),
        &proj,
        None,
        &dir.join("never_built"),
        "demo",
        &PackKey::generate(),
    );

    assert!(matches!(result, Err(PackageError::NoBinary(_))));
}

/// Developing the engine itself, or a project that needs nothing from it.
#[test]
fn packaging_works_without_an_engine_root() {
    let dir = tmp("noengine");
    let key = PackKey::generate();
    let proj = dir.join("proj");
    project(&proj);

    let out = assemble(
        &BuildPreset::default(),
        &known(),
        &proj,
        None,
        &binary(&dir),
        "demo",
        &key,
    )
    .unwrap();

    let pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert!(pack.contains("assets/props/rock.glb"));
}

/// The one that matters on unix: a game copied without its executable
/// bit is a game nobody can start.
#[cfg(unix)]
#[test]
fn the_binary_stays_executable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tmp("exec");
    let out = run(&dir, &BuildPreset::default()).unwrap();

    let mode = std::fs::metadata(&out.binary).unwrap().permissions().mode();
    assert!(mode & 0o111 != 0, "the shipped game is not executable");
}

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

/// 🔴 The opposite case, and the one it would be easy to break by
/// widening the filter: `.rendersettings` is what the project *looks*
/// like and the renderer reads it at startup.
#[test]
fn render_settings_still_ship() {
    let dir = tmp("settingsship");
    let key = PackKey::generate();
    let proj = dir.join("proj");
    project(&proj);
    write(&proj.join("assets/project.rendersettings"), b"()");

    let out = assemble(
        &BuildPreset::default(),
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

/// 🔴 The bug this replaced a curated list to fix: a scene using the
/// engine's `suzanne.glb` shipped without it and rendered nothing. The
/// old filter copied `materials` and `meshes/primitives` — a list
/// borrowed from *vendoring*, which answers "what source does a project
/// need to build", not "what does this game draw".
#[test]
fn an_engine_asset_the_scene_uses_ships() {
    let dir = tmp("suzanne");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    // Outside `meshes/primitives`, which is exactly where the old list
    // stopped looking.
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

/// 🔴 Without this the game cannot know which scene it opens with, and
/// `main_scene` goes back to being a field nothing reads (#808).
///
/// The bootstrap looks for the manifest **beside the executable**,
/// before the asset system exists — so it must be a plain file there,
/// not an entry in the pack.
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
