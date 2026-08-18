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
        // The image loader claims it, and a texture is what a material
        // reaches for — a fixture without it cannot exercise the graph
        // this file exists to walk.
        "png",
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

/// An engine texture that only a material names.
const ENGINE_TEXTURE: &str = "33333333-0000-4000-8000-000000000003";

/// The reported bug, as files: the scene names a material, the material
/// names a texture, and nothing else mentions the texture.
fn chained(dir: &Path, key: &PackKey) -> Package {
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    write(
        &proj.join("assets/materials/floor.ron"),
        format!(r#"(base_color: (1,1,1,1), albedo: Some("{ENGINE_TEXTURE}"))"#).as_bytes(),
    );
    write(
        &proj.join("assets/materials/floor.ron.meta"),
        b"guid = \"44444444-0000-4000-8000-000000000004\"\n",
    );
    write(&eng.join("assets/textures/grid.png"), b"engine texture");
    write(
        &eng.join("assets/textures/grid.png.meta"),
        format!("guid = \"{ENGINE_TEXTURE}\"\n").as_bytes(),
    );
    let exe = binary(dir);
    assemble(
        &BuildPreset::default(),
        &known(),
        &proj,
        Some(&eng),
        &exe,
        "demo",
        key,
    )
    .expect("packaging should succeed")
}

/// 🔴 The bug: a texture named only by a material has to travel.
///
/// The scene names the material, so the material shipped and the texture
/// did not — and a missing guid is silent, so the game rendered the 1x1
/// white fallback and looked like a material somebody authored flat.
/// Reported from a build made for the handheld.
#[test]
fn a_texture_named_only_by_a_material_travels() {
    let dir = tmp("packager_chain");
    let key = PackKey::generate();
    let out = chained(&dir, &key);
    let mut pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert_eq!(
        pack.read("assets/textures/grid.png").unwrap(),
        b"engine texture",
        "the engine texture did not ship; the packager stopped one level short",
    );
    // And its identity card with it: a file with no `.meta` is present
    // on disk and absent from the engine.
    assert!(pack.read("assets/textures/grid.png.meta").is_ok());
}

/// 🔴 A cycle terminates.
///
/// Two prefabs naming each other is authorable — a door prefab
/// referencing the room it opens into — and a closure that re-queues
/// what it has already seen turns that into a build which never
/// finishes. The dedup is what makes this a test that returns rather
/// than one that hangs the suite.
#[test]
fn a_reference_cycle_terminates() {
    let dir = tmp("packager_cycle");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    let (a, b) = (
        "55555555-0000-4000-8000-000000000005",
        "66666666-0000-4000-8000-000000000006",
    );
    for (name, guid, points_at) in [("a", a, b), ("b", b, a)] {
        write(
            &proj.join(format!("assets/{name}.prefab")),
            format!(
                r#"(entities: [(components: [(fields: [("x", AssetRef(guid: Some("{points_at}")))])])])"#
            )
            .as_bytes(),
        );
        write(
            &proj.join(format!("assets/{name}.prefab.meta")),
            format!("guid = \"{guid}\"\n").as_bytes(),
        );
    }
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
    .expect("a cycle is authorable and must not stop the build");
    let mut pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert!(pack.read("assets/a.prefab").is_ok());
}

/// 🔴 And the engine's demos still stay behind.
///
/// The failure mode of a transitive closure is the opposite of the bug
/// it fixes: follow one reference too far and the pack grows back into
/// the engine's 13 MB of demo content. Reachability has to stay
/// reachability.
#[test]
fn the_closure_does_not_swallow_the_engine() {
    let dir = tmp("packager_bounded");
    let key = PackKey::generate();
    let out = chained(&dir, &key);
    let mut pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert!(
        pack.read("assets/meshes/demo.glb").is_err(),
        "an unreferenced engine demo shipped — the closure followed something it should not",
    );
}

/// An asset many things name is collected once.
///
/// Measured as a DIFFERENCE rather than a total: the fixture ships its
/// own files, so an absolute count is a number that has to be updated
/// whenever the fixture grows — and the first version of this test
/// asserted 8, got 14, and was measuring the fixture.
///
/// Three materials sharing one texture must cost exactly two files more
/// than one material sharing it — the extra `.ron` and its `.meta`. A
/// closure that queued the texture once per referrer would copy it
/// again each time.
#[test]
fn a_shared_asset_is_collected_once() {
    fn pack_with(materials: usize, tag: &str) -> usize {
        let dir = tmp(tag);
        let (proj, eng) = (dir.join("proj"), dir.join("engine"));
        project(&proj);
        engine(&eng);
        for index in 0..materials {
            write(
                &proj.join(format!("assets/materials/m{index}.ron")),
                format!(r#"(albedo: Some("{ENGINE_TEXTURE}"))"#).as_bytes(),
            );
            write(
                &proj.join(format!("assets/materials/m{index}.ron.meta")),
                format!("guid = \"7{index}777777-0000-4000-8000-000000000007\"\n").as_bytes(),
            );
        }
        write(&eng.join("assets/textures/grid.png"), b"engine texture");
        write(
            &eng.join("assets/textures/grid.png.meta"),
            format!("guid = \"{ENGINE_TEXTURE}\"\n").as_bytes(),
        );
        let exe = binary(&dir);
        assemble(
            &BuildPreset::default(),
            &known(),
            &proj,
            Some(&eng),
            &exe,
            "demo",
            &PackKey::generate(),
        )
        .expect("packaging should succeed")
        .assets
    }

    let one = pack_with(1, "packager_shared_one");
    let three = pack_with(3, "packager_shared_three");
    assert_eq!(
        three - one,
        4,
        "two extra materials cost {} files instead of 4, so the shared texture was \
         collected more than once",
        three - one,
    );
}

/// 🔴 The case that needs the recursion, and not just the roots.
///
/// The project's own files are all read as roots, so a PROJECT material
/// reaching an engine texture is found without following anything —
/// which is why the first version of the test above passed with the
/// closure removed entirely.
///
/// This is the chain that only a walk can resolve: a scene names an
/// ENGINE material, and that material — a file the project never
/// touches — names an ENGINE texture. Exactly the shape of the shipped
/// prototype pack, where `dark_texture_08.ron` lives beside its own png
/// in the engine's tree.
#[test]
fn a_chain_inside_the_engine_resolves() {
    let dir = tmp("packager_engine_chain");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);

    // The engine's material — named by the fixture's scene — points at
    // an engine texture nothing else mentions.
    write(
        &eng.join("assets/materials/default.ron"),
        format!(r#"(albedo: Some("{ENGINE_TEXTURE}"))"#).as_bytes(),
    );
    write(&eng.join("assets/textures/grid.png"), b"engine texture");
    write(
        &eng.join("assets/textures/grid.png.meta"),
        format!("guid = \"{ENGINE_TEXTURE}\"\n").as_bytes(),
    );

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
    .expect("packaging should succeed");
    let mut pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert_eq!(
        pack.read("assets/textures/grid.png").unwrap(),
        b"engine texture",
        "the texture is two references deep and only through the engine's own tree, so \
         nothing but following the material finds it",
    );
}

/// Extensions the packager reads looking for references.
///
/// The counterpart of [`OPAQUE_FORMATS`]: between the two, every format
/// this engine loads is accounted for.
const TEXT_FORMATS: [&str; 6] = [
    "ron",
    "scene",
    "prefab",
    "rendersettings",
    "buildpreset",
    "inputaction",
];

/// 🔴 A new asset format has to be classified, and nothing else forces
/// it.
///
/// The packager decides whether to read a file by extension: text is
/// searched for references, binary is skipped. Both answers are silent
/// when wrong — a binary read as text finds nothing and ships an
/// incomplete pack; a text file skipped does the same. Neither fails to
/// compile and neither logs.
///
/// So the test is the forcing function. Add a loader, and this fails
/// until its extension is named as one or the other. ⚠️ Today the
/// answer for every binary format is "references nothing", which is a
/// fact about this engine and not a law: a `.glb` carries geometry and
/// its material is assigned by the scene. When that stops being true,
/// the format moves to [`TEXT_FORMATS`] and the packager follows it.
#[test]
fn every_asset_format_is_classified() {
    use kooch_core::asset_loader::AssetServer;

    let mut server = AssetServer::new();
    // The four the asset plugin registers by hand, plus everything
    // declared with `register_asset!`.
    server.register_loader::<kooch_render::mesh::Mesh, _>(kooch_render::mesh::GltfMeshLoader);
    server.register_loader::<kooch_render::meshlet::MeshletMesh, _>(
        kooch_render::meshlet::MeshletMeshLoader,
    );
    server.register_loader::<kooch_render::texture::Image, _>(
        kooch_render::texture::ImageLoader::srgb(),
    );
    server.register_loader::<kooch_render::material::Material, _>(
        kooch_render::material::MaterialLoader,
    );
    kooch_ecs::scene::prefab::register_loader(&mut server);
    for registration in kooch_core::asset_registry::registered_asset_types() {
        (registration.register_loader)(&mut server);
    }

    let mut unclassified: Vec<String> = Vec::new();
    for (extension, type_name) in server.known_extensions() {
        let lower = extension.to_ascii_lowercase();
        let opaque = OPAQUE_FORMATS.contains(&lower.as_str());
        let text = TEXT_FORMATS.contains(&lower.as_str());
        if opaque == text {
            unclassified.push(format!("{lower} ({type_name})"));
        }
    }
    assert!(
        unclassified.is_empty(),
        "these formats are in neither OPAQUE_FORMATS nor TEXT_FORMATS, so the packager \
         guessed: {unclassified:?}. Decide whether a file of that type can name another \
         asset — if it can, the packager must read it, and if it cannot, reading it is \
         waste repeated once per asset.",
    );
}

/// 🔴 An asset only the game's code names still ships.
///
/// The walk collects what the game can REACH by reading files, and a
/// guid built in Rust — loaded by path, chosen from a table, assembled
/// from a string — is reachable by nothing. Unity answers this with
/// `Resources/`, Godot with export filters; this manifest answers it
/// with a list, because the assets in question usually live in the
/// ENGINE's tree where a project cannot put a folder.
#[test]
fn a_declared_asset_ships_without_being_named() {
    let dir = tmp("packager_declared");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    // `demo.glb` is the engine asset no document mentions — the fixture
    // ships it precisely to prove it stays behind.
    write(
        &proj.join("project.kooch"),
        br#"(
            name: "demo",
            version: "0.1.0",
            engine_version: "0.6.0",
            main_scene: None,
            window: (title: "demo", width: 1280, height: 720),
            build: (include: ["assets/meshes/demo.glb"]),
        )"#,
    );
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
    .expect("packaging should succeed");
    let mut pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert_eq!(
        pack.read("assets/meshes/demo.glb").unwrap(),
        b"12 MB of demo",
        "the manifest declared it and it did not ship",
    );
}

/// And a declared asset is a ROOT, so what it names comes too.
///
/// Declaring a material and then having to declare its three textures
/// as well would be a list that goes stale the first time somebody edits
/// the material.
#[test]
fn a_declared_asset_brings_what_it_references() {
    let dir = tmp("packager_declared_chain");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    // An engine material nothing names, pointing at an engine texture
    // nothing names either.
    write(
        &eng.join("assets/materials/hidden.ron"),
        format!(r#"(albedo: Some("{ENGINE_TEXTURE}"))"#).as_bytes(),
    );
    write(
        &eng.join("assets/materials/hidden.ron.meta"),
        b"guid = \"bbbbbbbb-0000-4000-8000-00000000000b\"\n",
    );
    write(&eng.join("assets/textures/grid.png"), b"engine texture");
    write(
        &eng.join("assets/textures/grid.png.meta"),
        format!("guid = \"{ENGINE_TEXTURE}\"\n").as_bytes(),
    );
    write(
        &proj.join("project.kooch"),
        br#"(
            name: "demo",
            version: "0.1.0",
            engine_version: "0.6.0",
            main_scene: None,
            window: (title: "demo", width: 1280, height: 720),
            build: (include: ["assets/materials/hidden.ron"]),
        )"#,
    );
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
    .expect("packaging should succeed");
    let mut pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert!(
        pack.read("assets/materials/hidden.ron").is_ok(),
        "the declared material did not ship",
    );
    assert_eq!(
        pack.read("assets/textures/grid.png").unwrap(),
        b"engine texture",
        "the declared material shipped without the texture it names, so a declaration \
         would have to list every asset underneath it by hand",
    );
}
