//! #758 — what a player receives, and what must never be deleted to
//! produce it.

use super::*;

use kooch_pack::Pack;

/// The extensions a loader claims, as the real allowlist would hand them over. The fixtures use
/// these, so the tests exercise the filter rather than bypassing it.
fn known() -> Vec<String> {
    [
        "glb",
        // The image loader claims it, and a texture is what a material reaches for — a fixture
        // without it cannot exercise the graph this file exists to walk.
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

/// Guids the fixtures use, so a scene can name an engine asset the way a real one does.
const ENGINE_MATERIAL: &str = "11111111-0000-4000-8000-000000000001";
const ENGINE_CUBE: &str = "22222222-0000-4000-8000-000000000002";

/// A project with a scene, an asset and its sidecar.
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
    run_on(dir, preset, Platform::Linux)
}

fn run_on(dir: &Path, preset: &BuildPreset, platform: Platform) -> Result<Package, PackageError> {
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    let exe = binary(dir);
    assemble(
        preset,
        platform,
        &known(),
        &proj,
        Some(&eng),
        &exe,
        "demo",
        &PackKey::generate(),
    )
}

mod layout;
mod shipping;
mod closure;
