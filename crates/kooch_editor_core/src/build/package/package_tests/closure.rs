//! Following references to what a scene needs: chains, cycles, formats, declared assets, loaders.

use super::*;

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
        Platform::Linux,
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
        Platform::Linux,
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
            Platform::Linux,
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
        Platform::Linux,
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
const TEXT_FORMATS: [&str; 7] = [
    "material",
    "scene",
    "prefab",
    "rendersettings",
    "buildpreset",
    // Ships rather than being authoring-only: a block generates its
    // mesh from this file at load, so a game that carries the component
    // and not the source draws nothing. It leaves once baking does.
    "block",
    "inputaction",
];

/// 🔴 A new asset format has to be classified, and nothing else forces it.
#[test]
fn every_asset_format_is_classified() {
    let server = every_loader();

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
        Platform::Linux,
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
#[test]
fn a_declared_asset_brings_what_it_references() {
    let dir = tmp("packager_declared_chain");
    let key = PackKey::generate();
    let (proj, eng) = (dir.join("proj"), dir.join("engine"));
    project(&proj);
    engine(&eng);
    // An engine material nothing names, pointing at an engine texture nothing names either.
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
        Platform::Linux,
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

/// 🔴 One extension, one type.
#[test]
fn no_two_loaders_claim_one_extension() {
    use std::collections::HashMap;

    let server = every_loader();
    let mut owners: HashMap<String, Vec<String>> = HashMap::new();
    for (extension, type_name) in server.known_extensions() {
        owners
            .entry(extension.to_ascii_lowercase())
            .or_default()
            .push(type_name.to_owned());
    }

    // `glb` and `gltf` are the documented exception: `Mesh` and `MeshletMesh` are two views of ONE
    // file, chosen by the type the caller asks for, and the scan wanting the meshlet is the right
    // answer. Two loaders reading DIFFERENT files is the bug.
    const TWO_VIEWS_OF_ONE_FILE: [&str; 2] = ["glb", "gltf"];

    let shared: Vec<String> = owners
        .iter()
        .filter(|(extension, _)| !TWO_VIEWS_OF_ONE_FILE.contains(&extension.as_str()))
        .filter(|(_, types)| types.len() > 1)
        .map(|(extension, types)| format!("{extension} -> {}", types.join(", ")))
        .collect();
    assert!(
        shared.is_empty(),
        "these extensions are claimed by more than one loader, so the \
         scan types a file by whichever registered first:\n  {}",
        shared.join("\n  "),
    );
}

/// Everything a packaged game can be asked to load: the four the asset plugin registers by hand,
/// plus every type declared with `register_asset!`.
fn every_loader() -> kooch_core::asset_loader::AssetServer {
    let mut server = kooch_core::asset_loader::AssetServer::new();
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
    server
}
