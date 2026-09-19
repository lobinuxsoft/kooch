//! GPU acceptance for the static cut as geometry (#452): a masked material whose alpha reads uv
//! alone is baked, contoured and cut into a mesh that ends where the alpha does.
//!
//! Run with:
//!   cargo test -p kooch_render --test alpha_trim

mod common;

use kooch_core::Guid;
use kooch_render::material::{Material, MaterialPipeline, Shader};
use kooch_render::mesh::{Mesh, MeshVertex};
use kooch_render::meshlet::{
    DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES, LodConfig, MeshletMesh, build_meshlets_lod_chain,
    trim,
};

/// A cut at half the uv's x, as the graph's alpha clip pin writes it.
const HALF: &str = "fn surface(input: SurfaceInput) -> SurfaceOutput {
    var out: SurfaceOutput;
    out.normal = normalize(input.world_normal);
    out.roughness = 1.0;
    out.alpha = input.uv.x;
    out.alpha_clip = 0.5;
    return out;
}
";

/// The same cut, taken at a time that never stands still.
const MOVING: &str = "fn surface(input: SurfaceInput) -> SurfaceOutput {
    var out: SurfaceOutput;
    out.normal = normalize(input.world_normal);
    out.roughness = 1.0;
    out.alpha = input.uv.x * fract(input.time);
    out.alpha_clip = 0.5;
    return out;
}
";

/// A unit quad whose uv spans its square.
fn quad() -> MeshletMesh {
    let corner = |x: f32, y: f32| MeshVertex {
        position: [x, y, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [x, y],
    };
    let mesh = Mesh::from_arrays(
        vec![
            corner(0.0, 0.0),
            corner(1.0, 0.0),
            corner(1.0, 1.0),
            corner(0.0, 1.0),
        ],
        vec![0, 1, 2, 0, 2, 3],
    );
    build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("a quad meshletises")
}

/// The quad cut against `source`'s own alpha, on a real device.
fn cut(source: &str) -> Option<MeshletMesh> {
    let (device, queue) = common::try_acquire_device()?;
    let mut materials = MaterialPipeline::new(&device, &queue);
    let shader = Guid::new_v4();
    let parsed = Shader::parse(source).expect("the surface parses");
    materials.add_shader(shader, &parsed);
    let mut look = Material::new([1.0, 1.0, 1.0, 1.0], 0.0, 1.0, 0.0);
    look.shader = Some(shader);
    let slot = materials.register(&queue, Guid::new_v4(), &look);
    trim::build(&device, &queue, &materials, slot, &quad()).ok()
}

/// 🔴 The point of the step: the geometry stops at the alpha, and it is plain geometry — nothing in
/// it depends on the material any more.
#[test]
fn a_still_mask_becomes_geometry() {
    let Some((device, _)) = common::try_acquire_device() else {
        eprintln!("no capable adapter; skipping");
        return;
    };
    drop(device);
    let cut = cut(HALF).expect("half the quad survives its own alpha");
    assert!(cut.total_triangle_count() >= 2, "{cut:?}");
    // The alpha rises with uv.x, so what survives a clip of 0.5 is the far half of the quad.
    let near = cut
        .vertices
        .iter()
        .map(|vertex| vertex.position[0])
        .fold(f32::MAX, f32::min);
    assert!(
        (0.45..0.55).contains(&near),
        "the cut mesh starts at x {near}, not the alpha's half",
    );
}

/// A shader the bake cannot stand in for is refused before it reaches the graph: `masks_still` is
/// what the scene walk asks.
#[test]
fn a_moving_mask_is_refused() {
    assert!(Shader::parse(HALF).unwrap().masks_still());
    assert!(!Shader::parse(MOVING).unwrap().masks_still());
}
