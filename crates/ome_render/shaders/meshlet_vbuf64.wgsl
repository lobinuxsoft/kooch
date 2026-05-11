// meshlet_vbuf64.wgsl — atomic R64 visibility-buffer rasterizer (#493).
//
// Mirrors Bevy's meshlet pipeline: instead of writing a packed u32 to a
// color attachment, the fragment writes a packed u64 to a storage R64Uint
// texture via `textureAtomicMax`. Combined with reversed-Z this turns the
// visibility buffer into a winner-takes-all atomic — the closest fragment
// wins per pixel deterministically, eliminating coplanar-meshlet z-fighting
// that the legacy R32Uint color-attachment path exhibits.
//
// Pack layout (matches Rust `pack_visibility` in `vbuf64.rs`):
//   bits [63:32] = bitcast<u32>(clip_position.z)   reversed-Z monotonic
//   bits [31:7]  = visible_slot                    25 bits (no +1 sentinel;
//                                                   we rely on depth_bits == 0
//                                                   under reversed-Z meaning
//                                                   "no fragment", same as
//                                                   Bevy)
//   bits [6:0]   = triangle_idx                    7 bits, ≤ 127
//
// Render pass setup:
//   - color_attachments: empty (this fragment doesn't return)
//   - depth_stencil: kept (early-Z elision still helps)
//   - bind group 4 binding 0: texture_storage_2d<r64uint, atomic>

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

struct MeshVertexStored {
    position: array<f32, 3>,
    normal: array<f32, 3>,
    uv: array<f32, 2>,
}

struct MeshletDescriptor {
    vertex_offset: u32,
    triangle_offset: u32,
    vertex_count: u32,
    triangle_count: u32,
    aabb_min: vec3<f32>,
    parent_meshlet_index: u32,
    aabb_max: vec3<f32>,
    lod_error: f32,
    bounds_center: vec3<f32>,
    bounding_radius: f32,
    cone_apex: vec3<f32>,
    cone_cutoff: f32,
    cone_axis: vec3<f32>,
    group_index: u32,
    children_group_index: u32,
    lod_level: u32,
    _pad4: u32,
    _pad5: u32,
}

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

@group(0) @binding(0) var<uniform> camera: CameraUniforms;

@group(1) @binding(0) var<storage, read> vertices: array<MeshVertexStored>;
@group(1) @binding(1) var<storage, read> meshlet_vertices: array<u32>;
@group(1) @binding(2) var<storage, read> meshlet_triangles: array<u32>;
@group(1) @binding(3) var<storage, read> descriptors: array<MeshletDescriptor>;

@group(2) @binding(0) var<storage, read> visible_meshlets: array<u32>;

@group(3) @binding(0) var<storage, read> instances: array<MeshInstance>;

@group(4) @binding(0) var vbuf64: texture_storage_2d<r64uint, atomic>;

// #454 — Per-pixel R32Uint atomic accumulator backing the
// TriangleDensity and Overdraw heatmaps. Same texture, modal
// semantics — the mode word in `.x` decides which metric the
// fragment increments this frame:
//
//   0u — disabled (production rendering). Skip the atomicAdd
//        entirely; the accumulator stays zero across the frame.
//   1u — TriangleDensity. Every fragment increments by 1, so the
//        final value is the number of cluster fragments that
//        contributed to the pixel (winner or loser).
//   2u — Overdraw. Only fragments that win the vbuf atomicMax
//        increment, so the final value is the number of times
//        the depth buffer was overwritten — the "wasted shading"
//        signal Nanite-style references talk about.
//
// Branch is uniform across the wavefront (one UBO fetch +
// predicted branch per fragment) so the cost on production paths
// is amortised. Reused accumulator avoids paying a second R32Uint
// texture's VRAM (~4 MB at 1280×720) for a metric that is never
// viewed in parallel with TriangleDensity.
@group(5) @binding(0) var density_accumulator: texture_storage_2d<r32uint, atomic>;
@group(5) @binding(1) var<uniform> density_mode: vec4<u32>;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) @interpolate(flat) packed_id: u32,
}

fn fetch_local_vertex_index(byte_offset: u32) -> u32 {
    let word_idx = byte_offset / 4u;
    let byte_in_word = byte_offset & 3u;
    let packed = meshlet_triangles[word_idx];
    return (packed >> (byte_in_word * 8u)) & 0xffu;
}

@vertex
fn vs_vbuf64_scene(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VsOut {
    let packed_visible = visible_meshlets[instance_index];
    let inst_id = packed_visible >> 16u;
    let meshlet_id = packed_visible & 0xffffu;

    let inst = instances[inst_id];
    let desc = descriptors[meshlet_id];

    let triangle_idx = vertex_index / 3u;
    let corner_idx = vertex_index % 3u;

    var out: VsOut;
    if (triangle_idx >= desc.triangle_count) {
        // NaN clip position → primitive is culled by the rasterizer.
        // Mirror Bevy's `dummy_vertex` so out-of-range invocations
        // never reach `fs_vbuf64_scene`.
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.packed_id = 0u;
        return out;
    }

    let byte_offset = desc.triangle_offset + triangle_idx * 3u + corner_idx;
    let local_vertex_idx = fetch_local_vertex_index(byte_offset);
    let global_vertex_idx = meshlet_vertices[desc.vertex_offset + local_vertex_idx];
    let v = vertices[global_vertex_idx];

    let pos = vec3<f32>(v.position[0], v.position[1], v.position[2]);
    let world_pos = inst.transform * vec4<f32>(pos, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    // visible_slot encodes (instance_id, meshlet_idx) via one
    // indirection through visible_meshlets[] in the deferred shader,
    // matching the R32 scene path. No +1 offset: we use depth_bits == 0
    // (reversed-Z far plane) as the "no fragment" sentinel, like Bevy.
    let visible_slot = instance_index;
    out.packed_id = (visible_slot << 7u) | (triangle_idx & 0x7Fu);
    return out;
}

@fragment
fn fs_vbuf64_scene(input: VsOut) {
    let pixel = vec2<u32>(u32(input.clip_position.x), u32(input.clip_position.y));
    let depth_bits = bitcast<u32>(input.clip_position.z);
    let visibility = (u64(depth_bits) << 32u) | u64(input.packed_id);

    // WGSL's `textureAtomicMax` is statement-only (no return value,
    // unlike `atomicMax` on a storage buffer), so for Overdraw we
    // peek the current best with a non-atomic `textureLoad` before
    // performing the atomic update. The peek is racy in the strict
    // sense — another fragment can stomp the slot between the load
    // and the atomicMax — but the consequence is only a per-pixel
    // ±1 wobble in the heatmap, never a correctness issue for the
    // vbuf64 itself. The atomicMax remains the single source of
    // truth for shading; the heatmap is a calibration overlay.
    let mode = density_mode.x;
    var pre = u64(0);
    if (mode == 2u) {
        pre = textureLoad(vbuf64, pixel).x;
    }
    textureAtomicMax(vbuf64, pixel, visibility);

    if (mode == 1u) {
        // TriangleDensity: every fragment contributes one count.
        textureAtomicAdd(density_accumulator, pixel, 1u);
    } else if (mode == 2u) {
        // Overdraw: count only fragments that win the depth race —
        // those rewrote the visibility buffer with a closer hit, so
        // every previous shading contribution for that pixel is
        // "wasted". A pixel with high overdraw count is one where
        // the engine rasterized many cluster fragments only to
        // throw all but the last one away.
        if (visibility > pre) {
            textureAtomicAdd(density_accumulator, pixel, 1u);
        }
    }
}
