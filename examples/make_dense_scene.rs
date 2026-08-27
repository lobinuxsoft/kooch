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
//! This lays out mostly cubes (1 meshlet each) around a handful of dragons
//! (~4700 meshlets each). With the defaults:
//!
//! ```text
//! 600 cubes + 8 dragons = 608 instances
//! dispatched: 608 × 4700 = 2,857,600 threads
//! useful:     600 × 1 + 8 × 4700 = 38,200
//! waste:      ~75×
//! ```
//!
//! Raise `--dragons` and the ratio improves; raise `--cubes` and it gets
//! worse. That asymmetry *is* the bug: adding cheap objects should not cost
//! more per object.
//!
//! # Use
//!
//! ```text
//! cargo run --example make_dense_scene --features editor,physics,testing -- <parent>
//! cargo run --example make_dense_scene --features editor,physics,testing -- <parent> 2000 4 128
//! ```
//!
//! The arguments are `<parent> [cubes] [dragons] [lights] [spacing]`.
//!
//! 🔴 `spacing` is how the sun's CHAIN gets exercised. Level 0 of the
//! clipmap spans 1.3 m and level 16 spans 84 km, and everything that
//! changes across it — the texel a shadow is quantised to, the bias
//! derived from that texel, which level the reader stops at — is one
//! constant in a scene 190 m wide. `-- <parent> 2000 24 64 30` puts the
//! same objects over 1.4 km and walks eight levels instead of two.
//!
//! # The lights MOVE, and that is the point
//!
//! A sun, plus `lights` pivots each carrying a point or a spot light at
//! the end of an arm, all of them casting and all of them turning.
//!
//! 🔴 Static lights are the case the shadow page cache handles for
//! free: their pages are drawn once and every later frame is a hit. The
//! budget was cleared on a scene whose lights did not move, and the
//! roadmap records that the moving case has never been measured on the
//! device. This scene is that case.
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

/// Spacing between instances, in world units, when nobody says
/// otherwise. Wide enough that the meshes do not intersect.
///
/// 🔴 Overridable because DISTANCE is its own test. The sun's clipmap
/// spans 1.3 m at level 0 and 84 km at level 16, and every property
/// that changes across that chain — the texel a shadow is quantised
/// to, the bias derived from it, which level the reader stops at — is
/// invisible in a scene 190 m across. Spreading the same object count
/// over kilometres costs nothing and is the only way to walk the
/// chain.
const SPACING: f32 = 3.0;

/// How far a light orbits from its pivot, and how fast.
///
/// The radius is over a page of the sun's finest clipmap level at the
/// default extent, so an orbiting light crosses page boundaries rather
/// than jittering inside one — crossing is what invalidates.
const ORBIT_RADIUS: f32 = 6.0;
const ORBIT_DEGREES: f32 = 45.0;

fn main() {
    let engine_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut args = std::env::args().skip(1);
    let parent = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let cubes: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(600);
    let dragons: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(8);
    let lights: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(64);
    let spacing: f32 = args
        .next()
        .and_then(|a| a.parse().ok())
        .unwrap_or(SPACING)
        .max(0.1);

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
    let document = scene(&assets, cubes, dragons, lights, spacing);
    // Mesh instances only. The camera and the sky are entities too, but
    // they carry no `MeshRenderer`, so they never reach the cull — and
    // counting them would overstate the figure this scene exists to show.
    let instances = cubes + dragons;
    document
        .save(&root.join(kooch_core::scene_paths::DEFAULT_SCENE_REL_PATH))
        .expect("writing the scene");

    // The arithmetic the scene exists to make visible. Printed rather than
    // left implicit so the number to beat is on screen before the editor
    // opens.
    // ⚠️ Approximate, and it has to be: the builder does not produce the
    // same count twice. Six imports of this exact file measured 4598 to
    // 4931 — see #984. Printed as a scale, not as a figure to subtract
    // one run's from another's.
    const DRAGON_MESHLETS: usize = 4700;
    let dispatched = instances * DRAGON_MESHLETS;
    let useful = cubes + dragons * DRAGON_MESHLETS;

    println!("\nproject ready: {}\n", root.display());
    println!(
        "  {cubes} cubes + {dragons} dragons = {instances} mesh instances \
         ({} entities with the camera, sky, sun and lights)",
        document.entities.len(),
    );
    println!(
        "  {:.0} m across at {spacing:.1} m spacing",
        (document.entities.len() as f64).sqrt().ceil() * spacing as f64,
    );
    println!(
        "  {lights} orbiting lights ({} point + {} spot), all casting",
        lights.div_ceil(2),
        lights / 2,
    );
    println!("  dispatched: {instances} × {DRAGON_MESHLETS} = {dispatched} threads");
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
    dragon: Guid,
}

impl Assets {
    fn read(engine_root: &Path) -> Self {
        Self {
            cube: guid_of(engine_root, "assets/meshes/primitives/cube.glb.meta"),
            dragon: guid_of(engine_root, "assets/meshes/dragon.glb.meta"),
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

/// Cubes on a grid, dragons spread through it, one camera far enough back
/// to hold the whole thing.
fn scene(
    assets: &Assets,
    cubes: usize,
    dragons: usize,
    lights: usize,
    spacing: f32,
) -> SceneDocument {
    let total = cubes + dragons;
    // Square-ish grid, so the camera distance below stays predictable
    // whatever the counts are.
    let columns = (total as f64).sqrt().ceil().max(1.0) as usize;
    let extent = columns as f32 * spacing;

    let mut entities = Vec::with_capacity(total + lights * 2 + 3);
    entities.push(camera(Vec3::new(0.0, extent * 0.45, extent * 0.9)));
    entities.push(sky());
    entities.push(sun());

    for index in 0..total {
        let column = index % columns;
        let row = index / columns;
        let position = Vec3::new(
            (column as f32 - columns as f32 * 0.5) * spacing,
            0.0,
            (row as f32 - columns as f32 * 0.5) * spacing,
        );

        // Dragons spread evenly rather than clumped at one end: a cluster
        // of them in one corner would be culled away as a group and the
        // frame would not show what it costs to keep them.
        let stride = if dragons == 0 {
            usize::MAX
        } else {
            total / dragons.max(1)
        };
        let is_dragon =
            dragons > 0 && index % stride.max(1) == 0 && index / stride.max(1) < dragons;

        entities.push(if is_dragon {
            instance(&format!("Dragon {index}"), assets.dragon, position, 1.0)
        } else {
            instance(&format!("Cube {index}"), assets.cube, position, 1.0)
        });
    }

    // The lights, spread over the SAME grid the meshes cover so every
    // one of them has something to cast onto. Spread rather than
    // clustered for the reason the dragons are: a clump culls as one.
    //
    // 🔴 Alternating point and spot, because they take different paths
    // through the page code — a point owns six cube faces and a spot
    // one frustum — and a scene with only one kind leaves half the
    // rasteriser unmeasured.
    for index in 0..lights {
        let angle = index as f32 / lights.max(1) as f32 * std::f32::consts::TAU;
        // Two turns of a spiral, so the lights do not land on one ring
        // at one distance from the camera.
        let reach = extent
            * 0.5
            * ((index as f32 / lights.max(1) as f32) * 2.0)
                .fract()
                .max(0.15);
        let pivot_at = Vec3::new(angle.cos() * reach, 2.5, angle.sin() * reach);

        let pivot_index = entities.len();
        entities.push(pivot(&format!("Light Pivot {index}"), pivot_at));
        entities.push(orbiting_light(
            &format!("Light {index}"),
            pivot_index,
            index % 2 == 0,
        ));
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

/// The sun. One per scene, and the consumer that actually rasterises
/// — the clipmap is its own, and every caster in its column pairs with
/// every page of it.
fn sun() -> EntityDescription {
    EntityDescription {
        name: "Sun".to_owned(),
        parent_index: None,
        parent: None,
        components: vec![
            name_of("Sun"),
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::transform::Transform>(),
                fields: vec![(
                    "position".to_owned(),
                    ReflectValue::Vec3(Vec3::new(0.0, 40.0, 0.0)),
                )],
            },
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::directional_light::DirectionalLight>(),
                fields: vec![
                    ("active".to_owned(), ReflectValue::Bool(true)),
                    ("intensity".to_owned(), ReflectValue::F32(2000.0)),
                    ("cast_shadows".to_owned(), ReflectValue::Bool(true)),
                ],
            },
        ],
    }
}

/// A turning pivot. Carries no light itself — the light hangs off it at
/// [`ORBIT_RADIUS`], so turning the pivot MOVES the light rather than
/// merely rotating it in place.
fn pivot(label: &str, position: Vec3) -> EntityDescription {
    EntityDescription {
        name: label.to_owned(),
        parent_index: None,
        parent: None,
        components: vec![
            name_of(label),
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::transform::Transform>(),
                fields: vec![("position".to_owned(), ReflectValue::Vec3(position))],
            },
            // 🔴 The engine's own, behind the `testing` feature — which
            // is why this example needs that feature to build. A scene
            // opened by a project compiled without it gets still
            // lights: an unregistered component is dropped on load, not
            // an error, so nothing says the benchmark stopped moving.
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::testing::spin::Spin>(),
                fields: vec![
                    ("axis".to_owned(), ReflectValue::Vec3(Vec3::Y)),
                    ("degrees".to_owned(), ReflectValue::F32(ORBIT_DEGREES)),
                ],
            },
        ],
    }
}

/// One light, parented to a pivot so it orbits.
///
/// `point` picks which kind; see the loop in [`scene`] for why the two
/// alternate.
fn orbiting_light(label: &str, pivot_index: usize, point: bool) -> EntityDescription {
    let transform = ComponentDescription {
        type_name: type_name_of::<kooch_ecs::transform::Transform>(),
        fields: vec![(
            "position".to_owned(),
            ReflectValue::Vec3(Vec3::new(ORBIT_RADIUS, 0.0, 0.0)),
        )],
    };
    let light = if point {
        ComponentDescription {
            type_name: type_name_of::<kooch_ecs::point_light::PointLight>(),
            fields: vec![
                ("active".to_owned(), ReflectValue::Bool(true)),
                ("intensity".to_owned(), ReflectValue::F32(8000.0)),
                ("range".to_owned(), ReflectValue::F32(12.0)),
                ("cast_shadows".to_owned(), ReflectValue::Bool(true)),
            ],
        }
    } else {
        ComponentDescription {
            type_name: type_name_of::<kooch_ecs::spot_light::SpotLight>(),
            fields: vec![
                ("active".to_owned(), ReflectValue::Bool(true)),
                ("intensity".to_owned(), ReflectValue::F32(12000.0)),
                ("range".to_owned(), ReflectValue::F32(16.0)),
                ("cast_shadows".to_owned(), ReflectValue::Bool(true)),
            ],
        }
    };
    EntityDescription {
        name: label.to_owned(),
        parent_index: Some(pivot_index),
        parent: None,
        components: vec![name_of(label), transform, light],
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
