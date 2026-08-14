// visibility_buffer_resolve.wgsl — the R64 path's half of attribute
// reconstruction (#440).
//
// Everything below the visibility-buffer read is shared with the R32
// compute path and lives in `surface_reconstruct.wgsl`, which is
// concatenated alongside this file. What stays here is what is
// genuinely specific to this path: the 64-bit storage-texture binding,
// the frame uniforms, and the `slot << 7` packing (the R32 path adds one
// so that zero can mean background).
//
// WGSL has no #include, so this file is CONCATENATED in Rust ahead of
// each material shader — see `compose_material_shader`.

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

struct ScreenUniforms {
    // Always the FULL resolution, on every path. #825 shades at a lower
    // rate by dispatching fewer threads, not by lying about the screen:
    // the froxel lookup and the contact-shadow dither are functions of
    // the pixel coordinate, and both paths have to agree on what that
    // coordinate is.
    size: vec2<u32>,
    material_id: u32,
    debug_mode: u32,
    // #825 — pixels per shaded sample, per axis. 1 shades every pixel;
    // 2 shades one pixel of each 2x2 quad and an upsample pass fills the
    // rest. The fragment path only ever sees 1.
    shading_rate: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var vbuf64: texture_storage_2d<r64uint, read>;
@group(0) @binding(1) var<uniform> camera: CameraUniforms;
@group(0) @binding(2) var<uniform> screen: ScreenUniforms;

/// Full attribute reconstruction for the pixel at `frag_coord`.
fn resolve_vertex_output(frag_coord: vec4<f32>) -> VertexOutput {
    let packed_ids = u32(textureLoad(vbuf64, vec2<u32>(frag_coord.xy)).x);
    let visible_slot = packed_ids >> 7u;
    let tri_idx = packed_ids & 0x7Fu;
    return resolve_surface(visible_slot, tri_idx, frag_coord.xy);
}
