//! Generates a project with a flat terrain and one Field stamp per Atlas heightmap (#2052), in a
//! grid, so every shape the engine ships can be seen and walked over side by side.
//!
//! ```text
//! cargo run --example make_atlas_stamps --features editor,terrain -- /var/mnt/DATA
//! ```
//!
//! Each `assets/terrain/stamps/*.png` is baked at [`RESOLUTION`] texels into the project's
//! `assets/terrain`: every field in a scene shares one 128 MiB binding, and 54 maps at 2048² would
//! be 432 MiB.

use std::path::{Path, PathBuf};

use glam::{DVec3, Quat, Vec3};

use kooch_core::Guid;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::scene::{ComponentDescription, EntityDescription, SceneDocument};

/// 128 KiB a field; finer than the 0.5 m voxel over a 28 m stamp.
const RESOLUTION: u32 = 256;
const COLUMNS: usize = 9;
/// Metres between stamp centres.
const SPACING: f32 = 36.0;
/// The stamp box's half-extents: 28 m across, 8 m tall.
const HALF: Vec3 = Vec3::new(14.0, 4.0, 14.0);
/// How far the box's floor sits under the ground, so its walls never show.
const SINK: f32 = 1.0;

const FIELD_TYPE: &str = "kooch_terrain::field::Field";
const SHADER_TYPE: &str = "kooch_render::material::shader::Shader";

/// A plane base and nothing else: the stamps are the only relief.
const FLAT_GRAPH: &str = "// kind: terrain
// base: plane
/*KOOCH_GRAPH;(nodes:{1:(value:(node:PlaneBase),pos:(x:0.0,y:0.0),open:true),2:(value:(node:ShaderOutput(kind:\"terrain\")),pos:(x:340.0,y:0.0),open:true)},wires:[(out_pin:(node:1,output:0),in_pin:(node:2,input:0))])*/
// Written by the shader graph. Edit it there: saving the graph replaces this file.

fn planet(input: PlanetInput) -> f32 {
    let n0 = vec4<f32>(input.altitude);
    return n0.x;
}
";

fn main() {
    let engine_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let parent = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let name = "AtlasStamps";
    if parent.join(name).exists() {
        eprintln!(
            "{} already exists. Delete it yourself, or pass a different parent directory.",
            parent.join(name).display(),
        );
        std::process::exit(1);
    }
    let root = kooch_editor_core::project::create_project(name, &parent, &engine_root)
        .expect("creating the project");
    let out = root.join("assets/terrain");
    std::fs::create_dir_all(&out).expect("creating assets/terrain");

    let generator = Guid::new_v4();
    std::fs::write(out.join("flat.shgraph"), FLAT_GRAPH).expect("writing the graph");
    write_meta(&out.join("flat.shgraph"), generator, SHADER_TYPE);

    let mut sources: Vec<PathBuf> = std::fs::read_dir(engine_root.join("assets/terrain/stamps"))
        .expect("reading assets/terrain/stamps")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "png"))
        .collect();
    sources.sort();
    let mut fields = Vec::with_capacity(sources.len());
    for path in &sources {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("field");
        let bytes = std::fs::read(path).expect("reading a heightmap");
        let field = kooch_editor_core::height_field(&bytes, RESOLUTION)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let file = out.join(format!("{stem}.{}", kooch_terrain::FIELD_EXTENSION));
        std::fs::write(&file, field.to_bytes()).expect("writing a field");
        let guid = Guid::new_v4();
        write_meta(&file, guid, FIELD_TYPE);
        println!("baked {stem} {:?}", field.size);
        fields.push((stem.to_owned(), guid));
    }

    scene(generator, &fields)
        .save(&root.join(kooch_core::scene_paths::DEFAULT_SCENE_REL_PATH))
        .expect("writing the scene");
    println!("\nproject ready: {}", root.display());
    println!(
        "open it in the editor: {} stamps in a grid, in file-name order from the camera's far left.",
        fields.len()
    );
}

fn write_meta(path: &Path, guid: Guid, asset_type: &str) {
    let mut name = path.as_os_str().to_owned();
    name.push(".meta");
    std::fs::write(
        name,
        format!("guid = \"{guid}\"\nasset_type = \"{asset_type}\"\n"),
    )
    .expect("writing a sidecar");
}

fn scene(generator: Guid, fields: &[(String, Guid)]) -> SceneDocument {
    let rows = fields.len().div_ceil(COLUMNS);
    let width = (COLUMNS as f32 - 1.0) * SPACING;
    let depth = (rows as f32 - 1.0) * SPACING;
    let stamps = fields
        .iter()
        .enumerate()
        .map(|(i, (_, guid))| {
            let x = (i % COLUMNS) as f32 * SPACING - width * 0.5;
            let z = (i / COLUMNS) as f32 * SPACING - depth * 0.5;
            stamp(Vec3::new(x, HALF.y - SINK, z), Some(*guid))
        })
        .collect();
    let extent = width.max(depth) * 0.5 + SPACING;
    SceneDocument {
        id: Guid::new_v4(),
        name: "Atlas Stamps".to_owned(),
        version: "0.1.0".to_owned(),
        entities: vec![
            camera(DVec3::new(0.0, 70.0, f64::from(depth) * 0.5 + 90.0)),
            sky(),
            light(),
            terrain(generator, extent, stamps),
        ],
    }
}

fn terrain(generator: Guid, extent: f32, stamps: Vec<ReflectValue>) -> EntityDescription {
    let asset = |guid: Option<Guid>, asset_type: &str| ReflectValue::AssetRef {
        guid,
        asset_type: asset_type.to_owned(),
    };
    let fields = vec![
        ("radius", ReflectValue::F32(0.0)),
        ("extent", ReflectValue::F32(extent)),
        ("relief", ReflectValue::F32(2.0 * HALF.y)),
        ("depth", ReflectValue::F32(4.0)),
        ("voxel", ReflectValue::F32(0.5)),
        ("lod_distance", ReflectValue::F32(32.0)),
        ("generator", asset(Some(generator), SHADER_TYPE)),
        (
            "material",
            asset(None, "kooch_render::material::asset::Material"),
        ),
        (
            "stamps",
            ReflectValue::List {
                items: stamps,
                element: Box::new(stamp(Vec3::ZERO, None)),
            },
        ),
    ];
    entity(
        "Atlas terrain",
        vec![
            transform(DVec3::ZERO, Quat::IDENTITY),
            ComponentDescription {
                type_name: "kooch_terrain::component::Terrain".to_owned(),
                fields: fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
            },
        ],
    )
}

/// A Field stamp that adds `field` over a box of [`HALF`] centred at `position`.
fn stamp(position: Vec3, field: Option<Guid>) -> ReflectValue {
    ReflectValue::Struct(
        [
            ("enabled", ReflectValue::Bool(true)),
            ("shape", ReflectValue::U32(kooch_terrain::SHAPE_FIELD)),
            ("operation", ReflectValue::U32(0)),
            ("position", ReflectValue::Vec3(position)),
            ("turn", ReflectValue::F32(0.0)),
            ("radius", ReflectValue::F32(5.0)),
            ("size", ReflectValue::Vec3(HALF)),
            ("height", ReflectValue::F32(10.0)),
            ("tube", ReflectValue::F32(1.5)),
            (
                "volume",
                ReflectValue::AssetRef {
                    guid: None,
                    asset_type: SHADER_TYPE.to_owned(),
                },
            ),
            (
                "field",
                ReflectValue::AssetRef {
                    guid: field,
                    asset_type: FIELD_TYPE.to_owned(),
                },
            ),
            ("smooth", ReflectValue::F32(1.0)),
            ("opacity", ReflectValue::F32(1.0)),
            ("layer", ReflectValue::U32(1)),
            ("paint", ReflectValue::F32(0.0)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect(),
    )
}

fn camera(position: DVec3) -> EntityDescription {
    entity(
        "Camera",
        vec![
            transform(position, Quat::from_rotation_x(-0.5)),
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::perspective_camera::PerspectiveCamera>(),
                fields: vec![],
            },
        ],
    )
}

fn sky() -> EntityDescription {
    entity(
        "Sky",
        vec![
            transform(DVec3::ZERO, Quat::IDENTITY),
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::sky_renderer::SkyRenderer>(),
                fields: vec![],
            },
        ],
    )
}

fn light() -> EntityDescription {
    let rotation = Quat::from_euler(glam::EulerRot::XYZ, -0.9, 0.6, 0.0);
    entity(
        "Sun",
        vec![
            transform(DVec3::ZERO, rotation),
            ComponentDescription {
                type_name: type_name_of::<kooch_ecs::directional_light::DirectionalLight>(),
                fields: vec![],
            },
        ],
    )
}

fn transform(position: DVec3, rotation: Quat) -> ComponentDescription {
    ComponentDescription {
        type_name: type_name_of::<kooch_ecs::transform::Transform>(),
        fields: vec![
            ("position".to_owned(), ReflectValue::DVec3(position)),
            ("rotation".to_owned(), ReflectValue::Quat(rotation)),
        ],
    }
}

fn entity(label: &str, mut components: Vec<ComponentDescription>) -> EntityDescription {
    components.insert(
        0,
        ComponentDescription {
            type_name: type_name_of::<kooch_ecs::name::Name>(),
            fields: vec![("value".to_owned(), ReflectValue::String(label.to_owned()))],
        },
    );
    EntityDescription {
        name: label.to_owned(),
        parent_index: None,
        parent: None,
        components,
    }
}

fn type_name_of<T: 'static>() -> String {
    std::any::type_name::<T>().to_owned()
}
