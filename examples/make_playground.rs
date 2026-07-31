//! Generates a project the editor can open, with a visible physics scene.
//!
//! # Why this exists
//!
//! The editor opens to a launcher, and the launcher needs a project on
//! disk. Creating one through the UI means clicking, and there is no New
//! Scene button yet (#619), so there was no way to *see* the physics work
//! without building one by hand first.
//!
//! # Remote mode, not local
//!
//! Play belongs in the editor's own viewport, with a `WorldSnapshot` taken
//! before and restored on Stop. That is what `--remote` does: the project's
//! binary hosts the ECS — which is what lets the project's own components
//! and scripts compile — and the editor drives `Playing` over the wire and
//! draws the result in place.
//!
//! The local path still shells out to `cargo run -- --game`, building the
//! project and opening a second window with no snapshot (#633). These
//! instructions deliberately avoid it.
//!
//! # Use
//!
//! ```text
//! cargo run --example make_playground --features editor,physics -- /var/mnt/DATA
//! cd /var/mnt/DATA/PhysicsPlayground && cargo run -- --remote   # builds once
//! # then, in the editor: Open Remote
//! ```
//!
//! # Sizes come from `Transform.scale`
//!
//! Colliders are authored at unit size and scaled, so the mesh and the
//! collider cannot disagree: the solver folds the same scale into the shape
//! that the renderer folds into the mesh. Assumes the shipped `cube.glb` is
//! one unit across and `sphere.glb` half a unit in radius — if either is
//! not, everything is uniformly wrong by that factor, which is the kind of
//! wrong you see immediately.

use std::path::{Path, PathBuf};

use glam::Vec3;

use kooch_core::Guid;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::scene::{ComponentDescription, EntityDescription, SceneDocument};

fn main() {
    let engine_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let parent = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let name = "PhysicsPlayground";
    let root = parent.join(name);

    // Never deletes anything. An earlier version wiped an existing project
    // directory to make regeneration easy, which is a generator that can
    // destroy work someone did in the editor — and the whole point of this
    // is to give someone a place to work.
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
    scene(&assets)
        .save(&root.join(kooch_core::scene_paths::DEFAULT_SCENE_REL_PATH))
        .expect("writing the scene");

    println!("\nproject ready: {}\n", root.display());
    println!("Play runs in the editor's viewport, which means remote mode:\n");
    println!("  cd {} && cargo run -- --remote", root.display());
    println!("     (first build compiles the engine — once)\n");
    println!("  then in the editor: Open Remote\n");
    println!("Press Play there and the bodies fall in the viewport; Stop restores");
    println!("the authored world from a snapshot. Physics → Contacts draws what the");
    println!("solver holds — though that overlay does not work in the editor yet, #634.");
}

/// The engine-shipped assets the scene points at.
///
/// GUIDs are read from the `.meta` sidecars rather than written out: the
/// sidecar is where the asset database gets them, and a hardcoded GUID is
/// a reference that breaks silently when an asset is reimported.
struct Assets {
    cube: Guid,
    sphere: Guid,
    red: Guid,
    blue: Guid,
    yellow: Guid,
}

impl Assets {
    fn read(engine_root: &Path) -> Self {
        Self {
            cube: guid_of(engine_root, "assets/meshes/primitives/cube.glb.meta"),
            sphere: guid_of(engine_root, "assets/meshes/primitives/sphere.glb.meta"),
            red: guid_of(engine_root, "assets/materials/red.ron.meta"),
            blue: guid_of(engine_root, "assets/materials/blue_metal.ron.meta"),
            yellow: guid_of(engine_root, "assets/materials/emissive_yellow.ron.meta"),
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

/// One entity per thing worth looking at, all of them with a mesh.
///
/// Without meshes the viewport is a clear sky: colliders are numbers and
/// the collider gizmo only outlines the selection, so a scene of nine
/// bodies looked like nothing was there at all.
fn scene(assets: &Assets) -> SceneDocument {
    SceneDocument {
        id: Guid::new_v4(),
        name: "Physics Playground".to_owned(),
        version: "0.1.0".to_owned(),
        entities: vec![
            camera(Vec3::new(0.0, 8.0, 26.0)),
            sky(),
            light(),
            // The floor. Reports hard landings, so the contact-force path
            // has something to say.
            body(Spec {
                label: "Ground",
                position: Vec3::new(0.0, -1.0, 0.0),
                scale: Vec3::new(40.0, 2.0, 40.0),
                kind: Kind::Static,
                shape: Shape::Cube,
                mesh: assets.cube,
                material: assets.blue,
                extra: &[
                    ("collision_events", ReflectValue::Bool(true)),
                    ("contact_force_events", ReflectValue::Bool(true)),
                    ("contact_force_threshold", ReflectValue::F32(1.0)),
                ],
            }),
            // #618: both authored at 3 kg. Select them and read Mass —
            // before the fix the sphere weighed thirty-four.
            body(Spec {
                label: "Small cube (3 kg)",
                position: Vec3::new(-7.0, 9.0, 0.0),
                scale: Vec3::ONE,
                kind: Kind::Dynamic(3.0),
                shape: Shape::Cube,
                mesh: assets.cube,
                material: assets.red,
                extra: &[],
            }),
            body(Spec {
                label: "Big sphere (also 3 kg)",
                position: Vec3::new(-2.0, 9.0, 0.0),
                scale: Vec3::splat(4.0),
                kind: Kind::Dynamic(3.0),
                shape: Shape::Sphere,
                mesh: assets.sphere,
                material: assets.red,
                extra: &[],
            }),
            // #623: two strips differing only in friction. Drop something
            // on each and watch one slide.
            body(Spec {
                label: "Ice strip (friction 0.02)",
                position: Vec3::new(9.0, 0.1, -7.0),
                scale: Vec3::new(16.0, 0.2, 4.0),
                kind: Kind::Static,
                shape: Shape::Cube,
                mesh: assets.cube,
                material: assets.blue,
                extra: &[("friction", ReflectValue::F32(0.02))],
            }),
            body(Spec {
                label: "Grippy strip (friction 1.5)",
                position: Vec3::new(9.0, 0.1, -12.0),
                scale: Vec3::new(16.0, 0.2, 4.0),
                kind: Kind::Static,
                shape: Shape::Cube,
                mesh: assets.cube,
                material: assets.red,
                extra: &[("friction", ReflectValue::F32(1.5))],
            }),
            // #561: a trigger volume. Emissive so it reads as "not solid",
            // which is the one thing about a sensor worth seeing at a
            // glance — the cube above it falls straight through.
            body(Spec {
                label: "Trigger volume (sensor)",
                position: Vec3::new(3.0, 4.0, 0.0),
                scale: Vec3::new(4.0, 1.0, 4.0),
                kind: Kind::Static,
                shape: Shape::Cube,
                mesh: assets.cube,
                material: assets.yellow,
                extra: &[
                    ("sensor", ReflectValue::Bool(true)),
                    ("collision_events", ReflectValue::Bool(true)),
                ],
            }),
            body(Spec {
                label: "Falls through the trigger",
                position: Vec3::new(3.0, 10.0, 0.0),
                scale: Vec3::ONE,
                kind: Kind::Dynamic(1.0),
                shape: Shape::Cube,
                mesh: assets.cube,
                material: assets.red,
                extra: &[],
            }),
            // #560: the two halves of a hinge. The Joint's two entity
            // references have to be set in the Inspector — a scene file
            // cannot express them without persistent ids, which are only
            // assigned once something already points at an entity.
            body(Spec {
                label: "Door frame (hinge anchor)",
                position: Vec3::new(10.0, 6.0, 6.0),
                scale: Vec3::splat(0.5),
                kind: Kind::Static,
                shape: Shape::Cube,
                mesh: assets.cube,
                material: assets.blue,
                extra: &[],
            }),
            body(Spec {
                label: "Door leaf (add a Joint)",
                position: Vec3::new(11.5, 6.0, 6.0),
                scale: Vec3::ONE,
                kind: Kind::Dynamic(2.0),
                shape: Shape::Cube,
                mesh: assets.cube,
                material: assets.yellow,
                extra: &[],
            }),
        ],
    }
}

enum Kind {
    Static,
    Dynamic(f32),
}

#[derive(Clone, Copy)]
enum Shape {
    Cube,
    Sphere,
}

/// Everything one body needs, named rather than positional — eight
/// arguments in a row is how a mesh ends up on the wrong entity.
struct Spec<'a> {
    label: &'a str,
    position: Vec3,
    /// Sizes both the mesh and the collider, which is why they agree.
    scale: Vec3,
    kind: Kind,
    shape: Shape,
    mesh: Guid,
    material: Guid,
    extra: &'a [(&'a str, ReflectValue)],
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
                type_name: type_name_of::<kooch_ecs::transform::Transform>(),
                fields: vec![],
            },
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::sky_renderer::SkyRenderer>(),
                fields: vec![],
            },
        ],
    }
}

/// A directional light, so the meshes have shape rather than being flat
/// silhouettes.
fn light() -> EntityDescription {
    EntityDescription {
        name: "Sun".to_owned(),
        parent_index: None,
        parent: None,
        components: vec![
            name_of("Sun"),
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::transform::Transform>(),
                fields: vec![(
                    "rotation".to_owned(),
                    ReflectValue::Quat(glam::Quat::from_euler(glam::EulerRot::XYZ, -0.9, 0.6, 0.0)),
                )],
            },
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::directional_light::DirectionalLight>(),
                fields: vec![],
            },
        ],
    }
}

/// The asset type names the `.meta` sidecars carry.
const MESH_TYPE: &str = "kooch_render::meshlet::asset::MeshletMesh";
const MATERIAL_TYPE: &str = "kooch_render::material::asset::Material";

/// A body: transform, rigid body, unit collider, and a mesh to see it by.
fn body(spec: Spec<'_>) -> EntityDescription {
    use kooch_physics::components::{
        Collider, KIND_DYNAMIC, KIND_STATIC, RigidBody, SHAPE_CUBOID, SHAPE_SPHERE,
    };

    let (kind_value, mass) = match spec.kind {
        Kind::Static => (KIND_STATIC, 0.0),
        Kind::Dynamic(mass) => (KIND_DYNAMIC, mass),
    };
    // Unit dimensions: the scale does the sizing, for both the mesh and
    // the collider, so the two cannot drift apart.
    let mut collider_fields = vec![
        (
            "shape".to_owned(),
            ReflectValue::U32(match spec.shape {
                Shape::Cube => SHAPE_CUBOID,
                Shape::Sphere => SHAPE_SPHERE,
            }),
        ),
        (
            "half_extents".to_owned(),
            ReflectValue::Vec3(Vec3::splat(0.5)),
        ),
        ("radius".to_owned(), ReflectValue::F32(0.5)),
    ];
    for (field, value) in spec.extra {
        collider_fields.push(((*field).to_owned(), value.clone()));
    }

    EntityDescription {
        name: spec.label.to_owned(),
        parent_index: None,
        parent: None,
        components: vec![
            name_of(spec.label),
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::transform::Transform>(),
                fields: vec![
                    ("position".to_owned(), ReflectValue::Vec3(spec.position)),
                    ("scale".to_owned(), ReflectValue::Vec3(spec.scale)),
                ],
            },
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::mesh_renderer::MeshRenderer>(),
                fields: vec![
                    (
                        "mesh".to_owned(),
                        ReflectValue::AssetRef {
                            guid: Some(spec.mesh),
                            // The canonical type name, which is what the
                            // asset database records on the sidecar and
                            // what the picker filters on.
                            asset_type: MESH_TYPE.to_owned(),
                        },
                    ),
                    (
                        "material".to_owned(),
                        ReflectValue::AssetRef {
                            guid: Some(spec.material),
                            asset_type: MATERIAL_TYPE.to_owned(),
                        },
                    ),
                    ("visible".to_owned(), ReflectValue::Bool(true)),
                ],
            },
            ComponentDescription {
                type_name: type_name_of::<RigidBody>(),
                fields: vec![
                    ("kind".to_owned(), ReflectValue::U32(kind_value)),
                    ("mass".to_owned(), ReflectValue::F32(mass)),
                ],
            },
            ComponentDescription {
                type_name: type_name_of::<Collider>(),
                fields: collider_fields,
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
/// `std::any::type_name`, and a hand-written string goes stale the moment a
/// module moves — which `kooch_physics::components` did when it was split.
fn type_name_of<T: 'static>() -> String {
    std::any::type_name::<T>().to_owned()
}
