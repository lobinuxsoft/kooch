// resolve_material_depth.wgsl — pass 1 of the two-pass material path (#440).
//
// A fullscreen fragment pass that reads the atomic R64 visibility buffer
// and writes each covered pixel's `material_id` into a Depth16Unorm
// target, encoded as `f32(material_id) / 65535.0`. Pass 2 then binds this
// as a read-only depth attachment with `CompareFunction::Equal` so each
// per-material shading pass only touches the pixels assigned to it — a
// hardware depth test doing the material cull for free.
//
// vbuf64 packing (mirrors meshlet_deferred_r64.wgsl):
//   packed = (depth_bits << 32) | (visible_slot << 7 | tri_idx)
// `packed == 0` (reversed-Z far + cleared vbuf) is the background
// sentinel — those pixels `discard`, leaving the depth target cleared.
//
// Adapted from Bevy's resolve_render_targets.wgsl::resolve_material_depth.
// Our layout differs only in how instance/material are resolved:
// `visible_meshlets[slot] = (inst_id << 16) | meshlet_id`, then
// `instances[inst_id].material_id`.

struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    lod_force_level: i32,
    group_base: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var vbuf64: texture_storage_2d<r64uint, read>;
@group(0) @binding(1) var<storage, read> visible_meshlets: array<u32>;
@group(0) @binding(2) var<storage, read> instances: array<MeshInstance>;

struct FsInput {
    @builtin(position) position: vec4<f32>,
}

// Fullscreen triangle cover — same 3-vertex trick as meshlet_blit.wgsl.
// vi 0 → (-1,-1), vi 1 → (3,-1), vi 2 → (-1,3).
@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FsInput {
    var out: FsInput;
    let x = f32((vertex_index & 1u) << 2u) - 1.0;
    let y = f32((vertex_index & 2u) << 1u) - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_resolve_material_depth(in: FsInput) -> @builtin(frag_depth) f32 {
    let visibility = textureLoad(vbuf64, vec2<u32>(in.position.xy)).x;
    if ((visibility >> 32u) == 0lu) {
        discard;
    }
    let visible_slot = u32(visibility) >> 7u;
    let inst_id = visible_meshlets[visible_slot] >> 16u;
    let material_id = instances[inst_id].material_id;
    return f32(material_id) / 65535.0;
}
