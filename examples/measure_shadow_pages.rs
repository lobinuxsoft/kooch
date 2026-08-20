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

use std::path::Path;

use glam::{Mat4, Quat, Vec2, Vec3};

use kooch::kooch_ecs::reflect::ReflectValue;
use kooch::kooch_ecs::scene::SceneDocument;
use kooch::kooch_lighting::{ClusterGrid, ClusterSettings};
use kooch::kooch_render::shadow::{
    CensusCamera, CensusKind, CensusLight, ClipmapConfig, PageConfig, census,
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

fn as_bool(value: Option<&ReflectValue>) -> bool {
    matches!(value, Some(ReflectValue::Bool(true)))
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
    skipped: usize,
}

fn read(document: &SceneDocument, viewport: Vec2) -> Option<Frame> {
    let mut camera = None;
    let mut casting = Vec::new();
    let mut every = Vec::new();
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
                _ => {}
            }
        }
    }

    Some(Frame {
        camera: camera?,
        casting,
        every,
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
    println!(
        "{path}\n  {:.0}x{:.0}, grid {}x{}x{}, {} lights of which {} cast today{}",
        viewport.x,
        viewport.y,
        grid.dimensions.x,
        grid.dimensions.y,
        grid.dimensions.z,
        frame.every.len(),
        frame.casting.len(),
        if frame.skipped > 0 {
            format!(", {} skipped", frame.skipped)
        } else {
            String::new()
        }
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
            let config = PageConfig {
                page,
                virtual_size: 16384,
            };
            let out = census(config, clipmap, &grid, &frame.camera, lights);
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
    // a cell is the one whose texels match the screen. Printed rather
    // than asserted, because "it does not matter" is the kind of claim
    // this project measures.
    let sweep: Vec<u32> = [4096u32, 8192, 16384]
        .iter()
        .map(|&virtual_size| {
            census(
                PageConfig {
                    page: 128,
                    virtual_size,
                },
                clipmap,
                &grid,
                &frame.camera,
                &frame.every,
            )
            .resident()
        })
        .collect();
    println!("\n  virtual 4k/8k/16k at page 128, every light: {sweep:?} resident");

    // Which half of the bill is which. The sun is one light and a
    // clipmap; the locals are a hundred and a mip chain each, and the
    // pool is the same pages either way.
    let config = PageConfig {
        page: 128,
        virtual_size: 16384,
    };
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
    // 🔴 The floor this can never go below, and the number the rows
    // above have to be read against: one screen pixel needs one shadow
    // texel, so a frame cannot need fewer pages than its pixels divided
    // by a page's texels. Anything above it is pages part-used — and a
    // walk over froxels marks whole *volumes* of empty air, where a walk
    // over the depth buffer would mark only the surfaces that exist.
    let per_page = (config.page * config.page) as f64;
    let floor = (viewport.x as f64 * viewport.y as f64 / per_page).ceil();
    println!(
        "  {:<7} {:>2}         {floor:>6} floor    {:>7.1} MiB",
        "screen",
        1,
        floor * config.page_bytes() as f64 / (1024.0 * 1024.0)
    );
    for (label, lights) in [("sun", &sun), ("locals", &local)] {
        let out = census(config, clipmap, &grid, &frame.camera, lights);
        println!(
            "  {label:<7} {:>2} lights  {:>6} resident  {:>7.1} MiB",
            lights.len(),
            out.resident(),
            mib(out.bytes())
        );
    }

    // 🔴 Where the gap between those two comes from, predicted before it
    // was run: a froxel is a *volume*, and the logarithmic slices make a
    // far one tens of metres deep. Projected into the sun's plane that
    // depth becomes lateral spread, so one cell claims pages across
    // ground no surface occupies. If that is the cause, thinner slices
    // must collapse the count; if the cause were the cell's on-screen
    // width, they would barely move it.
    // 🔴 Two sweeps, because the number above is only worth reporting
    // if it is the area the frustum really claims rather than an
    // artefact of how the grid was diced. Both were run as predictions
    // and both refuted the mechanism they were testing, which is what
    // leaves the count standing:
    //
    // - **Slice thickness.** A froxel is a volume, and the logarithmic
    //   slices make a far one tens of metres deep; projected into the
    //   sun's plane that depth becomes lateral spread. 32x thinner
    //   slices moved the count 20 %. Not the mechanism.
    // - **Cell count.** If pages were barely shared between neighbours,
    //   residency would track the number of cells. Over a 20x range it
    //   is flat: the union converges, which is what a conservative
    //   marking pass is supposed to do.
    println!("\n  sun residency against slice thickness, everything else held:");
    for z_slices in [8u32, 24, 64, 256] {
        let settings = ClusterSettings {
            z_slices,
            ..ClusterSettings::default()
        };
        let out = census(
            config,
            clipmap,
            &ClusterGrid::new(&settings, viewport),
            &frame.camera,
            &sun,
        );
        println!("    {z_slices:>4} slices  {:>6} resident", out.resident());
    }
    println!("\n  sun residency against the grid's cell count:");
    for total in [576u32, 1152, 2304, 4608, 9216] {
        let settings = ClusterSettings {
            total,
            ..ClusterSettings::default()
        };
        let grid = ClusterGrid::new(&settings, viewport);
        let cells = grid.cluster_count();
        let out = census(config, clipmap, &grid, &frame.camera, &sun);
        println!(
            "    {cells:>6} cells  {:>6} resident  {:>6.2} pages per cell",
            out.resident(),
            out.resident() as f32 / cells as f32
        );
    }
}
