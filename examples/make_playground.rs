//! Generates a project the editor can open, with a physics scene in it.
//!
//! # Why this exists
//!
//! The editor opens to a launcher, and the launcher needs a project on
//! disk. Creating one through the UI means clicking, and there is no New
//! Scene button yet (#619) — so there was no way to *see* the physics work
//! without doing it by hand first. Every check up to now has been headless.
//!
//! `create_project` is a public function, so this is a dozen lines.
//!
//! # Use
//!
//! ```text
//! cargo run --example make_playground --features editor
//! OME_EDITOR_AUTO_OPEN=<the path it prints> cargo run -p ome_editor
//! ```
//!
//! The scene is laid out so every physics feature is visible from the
//! default camera and reachable by clicking. Press Play and watch; the
//! Physics menu in the viewport toolbar draws what the solver holds.

use std::path::PathBuf;

use glam::Vec3;

use ome_ecs::reflect::ReflectValue;
use ome_ecs::scene::{ComponentDescription, EntityDescription, SceneDocument};

fn main() {
    let engine_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let parent = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let name = "PhysicsPlayground";
    let root = parent.join(name);

    if root.exists() {
        std::fs::remove_dir_all(&root).expect("clearing the old playground");
        println!("removed the previous {}", root.display());
    }

    let root = ome_editor_core::project::create_project(name, &parent, &engine_root)
        .expect("creating the project");

    let scene_path = root.join("scenes/default.ome_scene");
    scene().save(&scene_path).expect("writing the scene");

    println!("\nproject ready: {}\n", root.display());
    println!("open it with:\n");
    println!(
        "  OME_EDITOR_AUTO_OPEN={} cargo run -p ome_editor\n",
        root.display()
    );
    println!("then press Play, and try Physics → Contacts in the viewport toolbar.");
}

/// One entity per thing worth looking at.
fn scene() -> SceneDocument {
    SceneDocument {
        id: ome_core::Guid::new_v4(),
        name: "Physics Playground".to_owned(),
        version: "0.1.0".to_owned(),
        entities: vec![
            // Looking down the -Z axis at the whole arrangement.
            camera(Vec3::new(0.0, 6.0, 18.0)),
            sky(),
            // The floor. Static, wide, and it reports hard landings so the
            // contact-force path has something to say.
            named_body(
                "Ground",
                Vec3::new(0.0, -1.0, 0.0),
                Kind::Static,
                Shape::Cuboid(Vec3::new(20.0, 1.0, 20.0)),
                &[
                    ("collision_events", ReflectValue::Bool(true)),
                    ("contact_force_events", ReflectValue::Bool(true)),
                    ("contact_force_threshold", ReflectValue::F32(1.0)),
                ],
            ),
            // #618: both are authored at 3 kg. Before the fix the big one
            // weighed thirty-four. Select them and read Mass.
            named_body(
                "Small cube (3 kg)",
                Vec3::new(-6.0, 8.0, 0.0),
                Kind::Dynamic(3.0),
                Shape::Cuboid(Vec3::splat(0.5)),
                &[],
            ),
            named_body(
                "Big sphere (also 3 kg)",
                Vec3::new(-2.0, 8.0, 0.0),
                Kind::Dynamic(3.0),
                Shape::Sphere(2.0),
                &[],
            ),
            // #623: two identical cubes on floors that differ only in
            // friction. Push them in Play and watch one stop sooner.
            named_body(
                "Ice patch (friction 0.02)",
                Vec3::new(6.0, 0.1, -6.0),
                Kind::Static,
                Shape::Cuboid(Vec3::new(8.0, 0.1, 2.0)),
                &[("friction", ReflectValue::F32(0.02))],
            ),
            named_body(
                "Grippy patch (friction 1.5)",
                Vec3::new(6.0, 0.1, -10.0),
                Kind::Static,
                Shape::Cuboid(Vec3::new(8.0, 0.1, 2.0)),
                &[("friction", ReflectValue::F32(1.5))],
            ),
            // #561: a trigger volume. It reports overlap and never pushes,
            // so the cube above it falls straight through.
            named_body(
                "Trigger volume (sensor)",
                Vec3::new(2.0, 3.0, 0.0),
                Kind::Static,
                Shape::Cuboid(Vec3::new(2.0, 0.5, 2.0)),
                &[
                    ("sensor", ReflectValue::Bool(true)),
                    ("collision_events", ReflectValue::Bool(true)),
                ],
            ),
            named_body(
                "Falls through the trigger",
                Vec3::new(2.0, 9.0, 0.0),
                Kind::Dynamic(1.0),
                Shape::Cuboid(Vec3::splat(0.5)),
                &[],
            ),
            // #560: a hinge. The two bodies are here; the Joint component
            // needs its two entity references set by hand, which is the
            // one thing a scene file cannot do without persistent ids —
            // drag them in the Inspector.
            named_body(
                "Door frame (hinge anchor)",
                Vec3::new(8.0, 5.0, 4.0),
                Kind::Static,
                Shape::Cuboid(Vec3::splat(0.25)),
                &[],
            ),
            named_body(
                "Door leaf (add a Joint)",
                Vec3::new(9.5, 5.0, 4.0),
                Kind::Dynamic(2.0),
                Shape::Cuboid(Vec3::splat(0.5)),
                &[],
            ),
        ],
    }
}

enum Kind {
    Static,
    Dynamic(f32),
}

enum Shape {
    Sphere(f32),
    Cuboid(Vec3),
}

fn camera(position: Vec3) -> EntityDescription {
    EntityDescription {
        name: "Camera".to_owned(),
        parent_index: None,
        parent: None,
        components: vec![
            name_of("Camera"),
            ComponentDescription {
                type_name: type_name_of::<ome_ecs::transform::Transform>(),
                fields: vec![("position".to_owned(), ReflectValue::Vec3(position))],
            },
            ComponentDescription {
                type_name: type_name_of::<ome_ecs::perspective_camera::PerspectiveCamera>(),
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
                type_name: type_name_of::<ome_ecs::transform::Transform>(),
                fields: vec![],
            },
            ComponentDescription {
                type_name: type_name_of::<ome_ecs::sky_renderer::SkyRenderer>(),
                fields: vec![],
            },
        ],
    }
}

/// A body plus a collider, with `extra` applied to the collider.
fn named_body(
    label: &str,
    position: Vec3,
    kind: Kind,
    shape: Shape,
    extra: &[(&str, ReflectValue)],
) -> EntityDescription {
    use ome_physics::components::{
        Collider, KIND_DYNAMIC, KIND_STATIC, RigidBody, SHAPE_CUBOID, SHAPE_SPHERE,
    };

    let (kind_value, mass) = match kind {
        Kind::Static => (KIND_STATIC, 0.0),
        Kind::Dynamic(mass) => (KIND_DYNAMIC, mass),
    };
    let mut collider_fields = vec![match shape {
        Shape::Sphere(radius) => ("radius".to_owned(), ReflectValue::F32(radius)),
        Shape::Cuboid(half) => ("half_extents".to_owned(), ReflectValue::Vec3(half)),
    }];
    collider_fields.push((
        "shape".to_owned(),
        ReflectValue::U32(match shape {
            Shape::Sphere(_) => SHAPE_SPHERE,
            Shape::Cuboid(_) => SHAPE_CUBOID,
        }),
    ));
    for (field, value) in extra {
        collider_fields.push(((*field).to_owned(), value.clone()));
    }

    EntityDescription {
        name: label.to_owned(),
        parent_index: None,
        parent: None,
        components: vec![
            name_of(label),
            ComponentDescription {
                type_name: type_name_of::<ome_ecs::transform::Transform>(),
                fields: vec![("position".to_owned(), ReflectValue::Vec3(position))],
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
        type_name: type_name_of::<ome_ecs::name::Name>(),
        fields: vec![("value".to_owned(), ReflectValue::String(label.to_owned()))],
    }
}

/// The name the component registry keys on.
///
/// Asked of the compiler rather than typed out: the registry uses
/// `std::any::type_name`, and a hand-written string goes stale the moment a
/// module moves — which `ome_physics::components` did when it was split.
fn type_name_of<T: 'static>() -> String {
    std::any::type_name::<T>().to_owned()
}
