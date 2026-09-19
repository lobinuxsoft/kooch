// masked_raster.wgsl — a masked material's raster (#452): each fragment asks the material's own
// `surface` for alpha and survives only at or above `alpha_clip`. The pixel is rebuilt exactly as
// the shading pass rebuilds it, so what is cut here is what would have shaded as cut.

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

// One per bin, at a dynamic offset: the material it draws and where its slots start.
struct ScreenUniforms {
    size: vec2<u32>,
    material_id: u32,
    bin: u32,
    mip_bias_scale: f32,
    time: f32,
    _pad: vec2<u32>,
}

// Only `camera_position` is read by the contract, but the name has to be `inti`.
struct IntiFrame {
    camera_position: vec3<f32>,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> screen: ScreenUniforms;
@group(0) @binding(2) var<uniform> inti: IntiFrame;
@group(0) @binding(3) var<storage, read> masked_slots: array<u32>;
@group(0) @binding(4) var<storage, read> masked_bases: array<u32>;

struct MaskedOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) slot: u32,
    @location(1) @interpolate(flat) triangle: u32,
}

@vertex
fn vs_masked(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance: u32,
) -> MaskedOut {
    let slot = masked_slots[masked_bases[screen.bin] + instance];
    let packed = visible_meshlets[slot];
    let inst = instances[packed >> 16u];
    let desc = descriptors[packed & 0xffffu];
    let triangle = vertex_index / 3u;

    var out: MaskedOut;
    out.slot = slot;
    out.triangle = triangle;
    // Past the meshlet's own triangles: a degenerate position the rasteriser drops.
    if (triangle >= desc.triangle_count) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let vertex = global_vertex_id(desc, triangle, vertex_index % 3u);
    out.position = camera.view_proj * corner_world_position(inst, vertex);
    return out;
}

fn masked_keeps(frag: MaskedOut) -> bool {
    let surf = resolve_surface(frag.slot, frag.triangle, frag.position.xy);
    let shaded = surface(surface_input(surf, frag.position.xy));
    return shaded.alpha >= shaded.alpha_clip;
}
