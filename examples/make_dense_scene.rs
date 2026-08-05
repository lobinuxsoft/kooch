//! Generates a scene dense enough for #689 to be visible.
//!
//! # What it is built to show
//!
//! The scene cull dispatches `instance_count × max_meshlets_per_mesh`
//! threads, where the max is over **every mesh in the pool**. So the shape
//! that hurts is not "many instances" — it is *many cheap instances while
//! one expensive asset exists*, because every cheap one is charged at the
//! expensive one's rate.
//!
//! This lays out mostly cubes (1 meshlet each) around a handful of skulls
//! (4393 meshlets each). With the defaults:
//!
//! ```text
//! 600 cubes + 8 skulls = 608 instances
//! dispatched: 608 × 4393 = 2,670,944 threads
//! useful:     600 × 1 + 8 × 4393 = 35,744
//! waste:      ~75×
//! ```
//!
//! Raise `--skulls` and the ratio improves; raise `--cubes` and it gets
//! worse. That asymmetry *is* the bug: adding cheap objects should not cost
//! more per object.
//!
//! # Use
//!
//! ```text
//! cargo run --example make_dense_scene --features editor,physics -- /var/mnt/DATA
//! cargo run --example make_dense_scene --features editor,physics -- /var/mnt/DATA 2000 4
//! ```
//!
//! Then open the project in the editor and read the perf HUD.
//!
//! # No physics
//!
//! Nothing here has a collider. The measurement is of the cull, and a
//! solver stepping six hundred bodies would put its own cost in the same
//! frame time.

use std::path::{Path, PathBuf};

use glam::Vec3;
use kooch_core::Guid;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::scene::{ComponentDescription, EntityDescription, SceneDocument};

/// Asset type name recorded on the `.meta` sidecar.
const MESH_TYPE: &str = "kooch_render::meshlet::asset::MeshletMesh";

/// Spacing between instances, in world units. Wide enough that the
/// meshes do not intersect at the scales used below.
const SPACING: f32 = 3.0;

fn main() {
    let engine_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut args = std::env::args().skip(1);
    let parent = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let cubes: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(600);
    let skulls: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(8);

    let name = "DenseScene";
    let root = parent.join(name);
    // Never deletes. A generator that wipes a directory is a generator
    // that can destroy work someone did in the editor.
    if root.exists() {
        eprintln!(
            "{} already exists. Delete it yourself, or pass a different parent directory.",
            root.display(),
        );
        std::process::exit(1);
    }

    let assets = Assets::read(&engine_root);
    let root = kooch_editor_core::project::create_project(name, &parent, &engine_root)
        .expect("creating the project");
    let document = scene(&assets, cubes, skulls);
    // Mesh instances only. The camera and the sky are entities too, but
    // they carry no `MeshRenderer`, so they never reach the cull — and
    // counting them would overstate the figure this scene exists to show.
    let instances = cubes + skulls;
    document
        .save(&root.join(kooch_core::scene_paths::DEFAULT_SCENE_REL_PATH))
        .expect("writing the scene");

    // The arithmetic the scene exists to make visible. Printed rather than
    // left implicit so the number to beat is on screen before the editor
    // opens.
    const SKULL_MESHLETS: usize = 4393;
    let dispatched = instances * SKULL_MESHLETS;
    let useful = cubes + skulls * SKULL_MESHLETS;

    println!("\nproject ready: {}\n", root.display());
    println!(
        "  {cubes} cubes + {skulls} skulls = {instances} mesh instances \
         ({} entities with the camera and sky)",
        document.entities.len(),
    );
    println!("  dispatched: {instances} × {SKULL_MESHLETS} = {dispatched} threads");
    println!("  useful:     {useful}");
    println!("  waste:      {:.0}×\n", dispatched as f64 / useful as f64);
    println!("  cd {} && cargo run -- --remote", root.display());
    println!("  then in the editor: Open Remote\n");
}

/// The engine-shipped assets the scene points at.
///
/// Read from the `.meta` sidecars rather than hardcoded: the sidecar is
/// where the asset database gets them, and a written-out GUID breaks
/// silently the day an asset is reimported.
struct Assets {
    cube: Guid,
    skull: Guid,
}

impl Assets {
    fn read(engine_root: &Path) -> Self {
        Self {
            cube: guid_of(engine_root, "assets/meshes/primitives/cube.glb.meta"),
            skull: guid_of(engine_root, "assets/meshes/scattering_skull.glb.meta"),
        }
    }
}

/// The `guid = "..."` line of a `.meta` sidecar.
fn guid_of(engine_root: &Path, relative: &str) -> Guid {
    let path = engine_root.join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let raw = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("guid")
                .and_then(|rest| rest.split('"').nth(1))
        })
        .unwrap_or_else(|| panic!("no guid in {}", path.display()));
    raw.parse()
        .unwrap_or_else(|e| panic!("bad guid in {}: {e}", path.display()))
}

/// Cubes on a grid, skulls spread through it, one camera far enough back
/// to hold the whole thing.
fn scene(assets: &Assets, cubes: usize, skulls: usize) -> SceneDocument {
    let total = cubes + skulls;
    // Square-ish grid, so the camera distance below stays predictable
    // whatever the counts are.
    let columns = (total as f64).sqrt().ceil().max(1.0) as usize;
    let extent = columns as f32 * SPACING;

    let mut entities = Vec::with_capacity(total + 2);
    entities.push(camera(Vec3::new(0.0, extent * 0.45, extent * 0.9)));
    entities.push(sky());

    for index in 0..total {
        let column = index % columns;
        let row = index / columns;
        let position = Vec3::new(
            (column as f32 - columns as f32 * 0.5) * SPACING,
            0.0,
            (row as f32 - columns as f32 * 0.5) * SPACING,
        );

        // Skulls spread evenly rather than clumped at one end: a cluster
        // of them in one corner would be culled away as a group and the
        // frame would not show what it costs to keep them.
        let stride = if skulls == 0 {
            usize::MAX
        } else {
            total / skulls.max(1)
        };
        let is_skull = skulls > 0 && index % stride.max(1) == 0 && index / stride.max(1) < skulls;

        entities.push(if is_skull {
            instance(&format!("Skull {index}"), assets.skull, position, 1.0)
        } else {
            instance(&format!("Cube {index}"), assets.cube, position, 1.0)
        });
    }

    SceneDocument {
        id: Guid::new_v4(),
        name: "Dense Scene".to_owned(),
        version: "0.1.0".to_owned(),
        entities,
    }
}

/// One mesh instance. No collider — see the module docs on why.
fn instance(label: &str, mesh: Guid, position: Vec3, scale: f32) -> EntityDescription {
    EntityDescription {
        name: label.to_owned(),
        parent_index: None,
        parent: None,
        components: vec![
            name_of(label),
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::transform::Transform>(),
                fields: vec![
                    ("position".to_owned(), ReflectValue::Vec3(position)),
                    ("scale".to_owned(), ReflectValue::Vec3(Vec3::splat(scale))),
                ],
            },
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::mesh_renderer::MeshRenderer>(),
                fields: vec![(
                    "mesh".to_owned(),
                    ReflectValue::AssetRef {
                        guid: Some(mesh),
                        asset_type: MESH_TYPE.to_owned(),
                    },
                )],
            },
        ],
    }
}

fn camera(position: Vec3) -> EntityDescription {
    EntityDescription {
        name: "Camera".to_owned(),
        parent_index: None,
        parent: None,
        components: vec![
            name_of("Camera"),
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::transform::Transform>(),
                fields: vec![("position".to_owned(), ReflectValue::Vec3(position))],
            },
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::perspective_camera::PerspectiveCamera>(),
                fields: vec![],
            },
        ],
    }
}

fn sky() -> EntityDescription {
    EntityDescription {
        name: "Sky".to_owned(),
        parent_index: None,
        parent: None,
        components: vec![
            name_of("Sky"),
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::sky_renderer::SkyRenderer>(),
                fields: vec![],
            },
        ],
    }
}

fn name_of(label: &str) -> ComponentDescription {
    ComponentDescription {
        type_name: type_name_of::<kooch_ecs::name::Name>(),
        fields: vec![("value".to_owned(), ReflectValue::String(label.to_owned()))],
    }
}

/// The name the component registry keys on.
///
/// Asked of the compiler rather than typed out: the registry uses
/// `std::any::type_name`, and a hand-written string goes stale the moment
/// a module moves.
fn type_name_of<T: 'static>() -> String {
    std::any::type_name::<T>().to_owned()
}
