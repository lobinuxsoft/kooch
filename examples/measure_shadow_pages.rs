//! How many pages a real frame would make resident (#866).
//!
//! #866 declines to pick a page size, a page count or an atlas format
//! from a whiteboard — *"the first task in this issue is a measurement,
//! not an allocation"* — and this is that measurement. It walks the
//! froxel grid the engine already builds, marks every page each cell
//! needs from each light that reaches it, and prints what the distinct
//! pages would cost.
//!
//! The number to read it against is **152 MiB**: today's cascade + spot
//! array (128) plus the point cubes (24), standing whether or not the
//! frame contains a shadow-casting light.
//!
//! ```bash
//! cargo run --example measure_shadow_pages -- <scene> [width] [height]
//! ```
//!
//! ⚠️ Local lights only, and root-level transforms only. A parented
//! light is reported as skipped rather than placed at its local
//! position, because a light in the wrong place is a page count that
//! looks like an answer.

use std::path::{Path, PathBuf};

use glam::{Mat4, Quat, Vec2, Vec3};

use kooch::kooch_core::Guid;
use kooch::kooch_ecs::reflect::ReflectValue;
use kooch::kooch_ecs::scene::SceneDocument;
use kooch::kooch_lighting::{ClusterGrid, ClusterSettings};
use kooch::kooch_render::mesh::parse_mesh_bytes_full;
use kooch::kooch_render::shadow::{
    CensusCamera, CensusFrame, CensusKind, CensusLight, ClipmapConfig, PageConfig, WorldBox, census,
};

/// What today's separate allocations cost, from `atlas.rs` and
/// `cube.rs`: 2048² over eight layers, plus 512² over six faces of four
/// lights, both at `Depth32Float`.
const TODAY_MIB: f64 = 152.0;

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// A component's field, if it is there and it is the right shape.
fn field<'a>(fields: &'a [(String, ReflectValue)], name: &str) -> Option<&'a ReflectValue> {
    fields.iter().find(|(key, _)| key == name).map(|(_, v)| v)
}

fn as_f32(value: Option<&ReflectValue>) -> Option<f32> {
    match value {
        Some(ReflectValue::F32(v)) => Some(*v),
        _ => None,
    }
}

fn as_vec3(value: Option<&ReflectValue>) -> Option<Vec3> {
    match value {
        Some(ReflectValue::Vec3(v)) => Some(*v),
        _ => None,
    }
}

/// The mesh a renderer points at.
fn mesh_guid(fields: &[(String, ReflectValue)]) -> Option<Guid> {
    match field(fields, "mesh") {
        Some(ReflectValue::AssetRef { guid, .. }) => *guid,
        _ => None,
    }
}

fn as_bool(value: Option<&ReflectValue>) -> bool {
    matches!(value, Some(ReflectValue::Bool(true)))
}

/// Every `*.meta` under `root`, as guid to the file it describes.
///
/// The asset database's job, done the cheap way: this runs headless with
/// no project open, and a `.meta` is two lines of TOML.
fn guid_map(root: &Path) -> Vec<(Guid, PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "meta") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            // 🔴 Parsed rather than compared as text. A `.meta` writes
            // the hyphenated form and `Guid`'s own `Display` writes the
            // simple one, so two spellings of the same asset never match
            // as strings — which is what made every mesh unresolvable
            // the first time this ran.
            let Some(guid) = text
                .lines()
                .find_map(|line| line.strip_prefix("guid = "))
                .and_then(|v| v.trim().trim_matches('"').parse::<Guid>().ok())
            else {
                continue;
            };
            out.push((guid, path.with_extension("")));
        }
    }
    out
}

/// One mesh instance, before its asset has been found.
struct Instance {
    guid: Guid,
    world_from_local: Mat4,
}

/// Every mesh instance's world box.
///
/// ⚠️ A mesh whose asset cannot be found contributes nothing rather than
/// a guessed box: a surface in the wrong place is a page count that
/// looks like an answer. The count of those is reported.
fn surfaces(instances: &[Instance], roots: &[PathBuf]) -> (Vec<WorldBox>, usize) {
    let map: Vec<(Guid, PathBuf)> = roots.iter().flat_map(|r| guid_map(r)).collect();
    let mut cache: Vec<(Guid, Option<(Vec3, Vec3)>)> = Vec::new();
    let mut out = Vec::new();
    let mut missing = 0usize;

    for instance in instances {
        let local = match cache.iter().find(|(g, _)| *g == instance.guid) {
            Some((_, bounds)) => *bounds,
            None => {
                let bounds = map
                    .iter()
                    .find(|(g, _)| *g == instance.guid)
                    .and_then(|(_, path)| {
                        let bytes = std::fs::read(path).ok()?;
                        let mesh = parse_mesh_bytes_full(&bytes, 1.0, path.parent()).ok()?;
                        Some((mesh.aabb.min, mesh.aabb.max))
                    });
                cache.push((instance.guid, bounds));
                bounds
            }
        };
        let Some((min, max)) = local else {
            missing += 1;
            continue;
        };
        // The eight corners, so a rotation does not shrink the box.
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            );
            let world = instance.world_from_local.transform_point3(corner);
            lo = lo.min(world);
            hi = hi.max(world);
        }
        out.push(WorldBox::new(lo, hi));
    }
    (out, missing)
}

/// The camera the scene marks active, and every light in it.
///
/// 🔴 Two light lists, not one, and that is the point of the run.
/// `casting` is what the engine draws **today** — and in
/// `many_lights.scene` that is four of a hundred point lights, because
/// four is how many cube slots there are. The pool exists to retire that
/// limit (#841 / #849 are superseded by it), so the number that decides
/// is what `every` costs.
struct Frame {
    camera: CensusCamera,
    casting: Vec<CensusLight>,
    every: Vec<CensusLight>,
    meshes: Vec<Instance>,
    skipped: usize,
}

fn read(document: &SceneDocument, viewport: Vec2) -> Option<Frame> {
    let mut camera = None;
    let mut casting = Vec::new();
    let mut every = Vec::new();
    let mut meshes = Vec::new();
    let mut skipped = 0usize;

    for entity in &document.entities {
        let position = entity
            .components
            .iter()
            .find(|c| c.type_name.ends_with("::Transform"))
            .and_then(|c| as_vec3(field(&c.fields, "position")));
        let rotation = entity
            .components
            .iter()
            .find(|c| c.type_name.ends_with("::Transform"))
            .and_then(|c| match field(&c.fields, "rotation") {
                Some(ReflectValue::Quat(q)) => Some(*q),
                _ => None,
            })
            .unwrap_or(Quat::IDENTITY);

        let scale = entity
            .components
            .iter()
            .find(|c| c.type_name.ends_with("::Transform"))
            .and_then(|c| as_vec3(field(&c.fields, "scale")))
            .unwrap_or(Vec3::ONE);

        for component in &entity.components {
            let tail = component.type_name.rsplit("::").next().unwrap_or_default();
            match tail {
                "PerspectiveCamera" if as_bool(field(&component.fields, "active")) => {
                    let Some(position) = position else { continue };
                    let fov = as_f32(field(&component.fields, "fov")).unwrap_or(60.0);
                    let near = as_f32(field(&component.fields, "near")).unwrap_or(0.1);
                    camera = Some(CensusCamera {
                        world_from_view: Mat4::from_rotation_translation(rotation, position),
                        clip_from_view:
                            kooch::kooch_render::projection::perspective_infinite_rh_reverse_z(
                                fov.to_radians(),
                                viewport.x / viewport.y.max(1.0),
                                near.max(0.01),
                            ),
                        viewport,
                    });
                }
                "PointLight" | "SpotLight" => {
                    if !as_bool(field(&component.fields, "active")) {
                        continue;
                    }
                    let (Some(position), Some(range)) =
                        (position, as_f32(field(&component.fields, "range")))
                    else {
                        skipped += 1;
                        continue;
                    };
                    if entity.parent_index.is_some() {
                        skipped += 1;
                        continue;
                    }
                    let light = if tail == "PointLight" {
                        CensusLight::point(position, range)
                    } else {
                        CensusLight::spot(position, range)
                    };
                    every.push(light);
                    if as_bool(field(&component.fields, "cast_shadows")) {
                        casting.push(light);
                    }
                }
                "DirectionalLight" => {
                    if !as_bool(field(&component.fields, "active"))
                        || !as_bool(field(&component.fields, "cast_shadows"))
                    {
                        continue;
                    }
                    // A directional light points where its transform
                    // does; the component carries no direction of its
                    // own.
                    let sun = CensusLight::sun(rotation * Vec3::NEG_Z);
                    every.push(sun);
                    casting.push(sun);
                }
                "MeshRenderer" => {
                    let Some(position) = position else { continue };
                    let Some(guid) = mesh_guid(&component.fields) else {
                        continue;
                    };
                    meshes.push(Instance {
                        guid,
                        world_from_local: Mat4::from_scale_rotation_translation(
                            scale, rotation, position,
                        ),
                    });
                }
                _ => {}
            }
        }
    }

    Some(Frame {
        camera: camera?,
        casting,
        every,
        meshes,
        skipped,
    })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        eprintln!("usage: measure_shadow_pages <scene> [width] [height]");
        std::process::exit(2);
    });
    let width: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1280.0);
    let height: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(720.0);
    let viewport = Vec2::new(width, height);

    let document = match SceneDocument::load(Path::new(&path)) {
        Ok(document) => document,
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    };
    let Some(frame) = read(&document, viewport) else {
        eprintln!("{path}: no active perspective camera");
        std::process::exit(1);
    };

    let grid = ClusterGrid::new(&ClusterSettings::default(), viewport);
    let clipmap = ClipmapConfig::default();

    // Where the meshes might live: the project the scene belongs to, and
    // the engine's own assets, which is where the primitives are.
    let mut roots: Vec<PathBuf> = Vec::new();
    for ancestor in Path::new(&path).ancestors() {
        if ancestor.file_name().is_some_and(|n| n == "assets") {
            roots.push(ancestor.to_path_buf());
        }
    }
    roots.push(PathBuf::from("assets"));
    let (surfaces, missing) = surfaces(&frame.meshes, &roots);

    let run = |config: PageConfig, lights: &[CensusLight], boxes: &[WorldBox]| {
        census(
            config,
            clipmap,
            &grid,
            &CensusFrame {
                camera: frame.camera,
                lights,
                surfaces: boxes,
            },
        )
    };

    println!(
        "{path}\n  {:.0}x{:.0}, grid {}x{}x{}, {} lights of which {} cast today, \
         {} mesh instances{}{}",
        viewport.x,
        viewport.y,
        grid.dimensions.x,
        grid.dimensions.y,
        grid.dimensions.z,
        frame.every.len(),
        frame.casting.len(),
        surfaces.len(),
        if missing > 0 {
            format!(", {missing} unresolved")
        } else {
            String::new()
        },
        if frame.skipped > 0 {
            format!(", {} skipped", frame.skipped)
        } else {
            String::new()
        }
    );

    let config = PageConfig {
        page: 128,
        virtual_size: 16384,
    };

    // 🔴 The comparison this run exists for. The left column marks every
    // cell of the frustum; the right marks only the cells a mesh passes
    // through. A froxel is a box of mostly empty air, and a page
    // allocated for air is a page no shadow ever reads.
    let sun: Vec<CensusLight> = frame
        .every
        .iter()
        .copied()
        .filter(|l| matches!(l.kind, CensusKind::Sun(_)))
        .collect();
    let local: Vec<CensusLight> = frame
        .every
        .iter()
        .copied()
        .filter(|l| !matches!(l.kind, CensusKind::Sun(_)))
        .collect();

    println!(
        "\n  page 128, virtual 16k, one shadow texel per screen pixel\n\n\
         {:<20} {:>9} {:>9} {:>9} {:>9} {:>7}",
        "", "cells", "volume", "surfaces", "MiB", "saved"
    );
    for (label, lights) in [
        ("the sun", &sun),
        ("100 local lights", &local),
        ("everything", &frame.every),
    ] {
        let volume = run(config, lights, &[]);
        let surface = run(config, lights, &surfaces);
        println!(
            "  {label:<18} {:>9} {:>9} {:>9} {:>9.1} {:>6.1}x",
            surface.cells(),
            volume.resident(),
            surface.resident(),
            mib(surface.bytes()),
            volume.resident() as f32 / surface.resident().max(1) as f32
        );
    }

    // The floor neither walk can go below: one screen pixel needs one
    // shadow texel, so a frame cannot need fewer pages than its pixels
    // divided by a page's texels.
    let per_page = (config.page * config.page) as f64;
    let floor = (viewport.x as f64 * viewport.y as f64 / per_page).ceil();
    println!(
        "  {:<18} {:>9} {:>9} {:>9} {:>9.1}",
        "the screen's floor",
        "-",
        "-",
        floor as u64,
        floor * config.page_bytes() as f64 / (1024.0 * 1024.0)
    );
    println!(
        "  {:<18} {:>9} {:>9} {:>9} {:>9.1}",
        "today, 5 casting", "-", "-", "-", TODAY_MIB
    );

    println!(
        "\n{:>6} {:>9} {:>10} {:>11} {:>9} {:>11}",
        "page", "casting", "resident", "cell/light", "MiB", "vs 152 MiB"
    );
    for page in [64u32, 128, 256] {
        for (label, lights) in [
            (frame.casting.len(), &frame.casting),
            (frame.every.len(), &frame.every),
        ] {
            let out = run(
                PageConfig {
                    page,
                    virtual_size: 16384,
                },
                lights,
                &surfaces,
            );
            let mib = mib(out.bytes());
            println!(
                "{page:>6} {label:>9} {:>10} {:>11} {mib:>9.1} {:>10.2}x",
                out.resident(),
                out.pairs(),
                mib / TODAY_MIB
            );
        }
    }

    // The virtual size is the level chain's ceiling, not its working
    // set: a bigger map only adds finer levels, and the level chosen for
    // a cell is the one whose texels match the screen.
    let sweep: Vec<u32> = [4096u32, 8192, 16384]
        .iter()
        .map(|&virtual_size| {
            run(
                PageConfig {
                    page: 128,
                    virtual_size,
                },
                &frame.every,
                &surfaces,
            )
            .resident()
        })
        .collect();
    println!("\n  virtual 4k/8k/16k at page 128, every light: {sweep:?} resident");

    // Two sweeps kept because both were run as predictions about the
    // volume walk's count and both refuted the mechanism they tested,
    // which is what leaves that count standing as an area rather than an
    // artefact of how the grid is diced.
    println!("\n  the sun's VOLUME residency against slice thickness:");
    for z_slices in [8u32, 24, 64, 256] {
        let settings = ClusterSettings {
            z_slices,
            ..ClusterSettings::default()
        };
        let out = census(
            config,
            clipmap,
            &ClusterGrid::new(&settings, viewport),
            &CensusFrame {
                camera: frame.camera,
                lights: &sun,
                surfaces: &[],
            },
        );
        println!("    {z_slices:>4} slices  {:>6} resident", out.resident());
    }
    println!("\n  the sun's VOLUME residency against the grid's cell count:");
    for total in [576u32, 2304, 9216] {
        let settings = ClusterSettings {
            total,
            ..ClusterSettings::default()
        };
        let inner = ClusterGrid::new(&settings, viewport);
        let cells = inner.cluster_count();
        let out = census(
            config,
            clipmap,
            &inner,
            &CensusFrame {
                camera: frame.camera,
                lights: &sun,
                surfaces: &[],
            },
        );
        println!(
            "    {cells:>6} cells  {:>6} resident  {:>6.2} pages per cell",
            out.resident(),
            out.resident() as f32 / cells as f32
        );
    }
}
