//! #758 — what a player receives, and what must never be deleted to
//! produce it.

use super::*;

use kooch_pack::Pack;

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

/// A project with a scene, two assets and their sidecars.
fn project(root: &Path) {
    write(&root.join("scenes/default.scene"), b"(entities: [])");
    write(&root.join("assets/props/rock.glb"), b"rock mesh");
    write(&root.join("assets/props/rock.glb.meta"), b"(guid: \"r\")");
}

/// An engine root with the two directories a game actually needs.
fn engine(root: &Path) {
    write(
        &root.join("assets/materials/default.ron"),
        b"engine material",
    );
    write(&root.join("assets/meshes/primitives/cube.glb"), b"cube");
    // A demo the vendor list deliberately leaves behind.
    write(&root.join("assets/meshes/demo.glb"), b"12 MB of demo");
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
    assert_eq!(out.binary.file_name().unwrap(), "demo");
    assert!(out.dir.join("scenes/default.scene").is_file());
    assert!(out.dir.join(PACK_FILE).is_file());
    assert_eq!(out.scenes, 1);
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
        &proj,
        Some(&eng),
        &exe,
        "demo",
        &key,
    )
    .unwrap();

    let mut pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert_eq!(
        pack.read("materials/default.ron").unwrap(),
        b"engine material"
    );
    assert_eq!(pack.read("meshes/primitives/cube.glb").unwrap(), b"cube");
    assert_eq!(pack.read("props/rock.glb").unwrap(), b"rock mesh");
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
        &proj,
        Some(&eng),
        &binary(&dir),
        "demo",
        &key,
    )
    .unwrap();

    let pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert!(!pack.contains("meshes/demo.glb"), "a demo model shipped");
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

    let out = assemble(
        &BuildPreset::default(),
        &proj,
        Some(&eng),
        &binary(&dir),
        "demo",
        &key,
    )
    .unwrap();

    assert_eq!(out.shadowed, vec!["materials/default.ron"]);
    let mut pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert_eq!(pack.read("materials/default.ron").unwrap(), b"mine");
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
        let result = assemble(&preset, &proj, None, &exe, "demo", &PackKey::generate());
        assert!(
            matches!(result, Err(PackageError::UnsafeOutput(_))),
            "packaging into {target:?} was allowed",
        );
    }
    // And nothing was taken with it.
    assert!(proj.join("assets/props/rock.glb").is_file());
    assert!(proj.join("scenes/default.scene").is_file());
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
        &proj,
        None,
        &binary(&dir),
        "demo",
        &key,
    )
    .unwrap();

    let pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert!(pack.contains("props/rock.glb"));
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
    assert!(shipped.iter().any(|name| *name == "props/rock.glb"));
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
        &proj,
        None,
        &binary(&dir),
        "demo",
        &key,
    )
    .unwrap();

    let pack = Pack::open(&out.pack.unwrap(), &key).unwrap();
    assert!(
        pack.contains("project.rendersettings"),
        "the project's look did not ship",
    );
}
