// motion_vectors.wgsl — where each pixel's surface was last frame (#481).
//
// The prerequisite every temporal technique shares: TAA, FSR 2+, DLSS,
// XeSS and motion blur all want depth, a jittered projection and this.
//
// CONCATENATED after `visibility_buffer_resolve.wgsl` and
// `surface_reconstruct.wgsl`, so the vbuf decode, the meshlet indexing
// and the perspective-correct barycentrics are the ones the shading path
// already uses, not a second implementation of them.
//
// # 🔴 Why this reconstructs the surface instead of reading the vbuf twice
//
// The obvious implementation asks "what was at this pixel last frame".
// That question has no answer under continuous LOD: a surface can change
// cluster between frames without moving a millimetre, and every LOD
// transition would then read as motion and smear.
//
// So the question asked here is the other one — **where was THIS point of
// THIS surface** — answered by transforming the same three vertices by
// the instance's previous matrix and interpolating with the same
// barycentrics. A LOD switch changes which triangle covers the pixel; it
// does not change where the surface was, so the vector stays zero on a
// static object. Bevy's meshlet path does exactly this
// (`visibility_buffer_resolve.wesl:152`).
//
// # Why it is a pass of its own
//
// Shading runs at half rate (#825) and a temporal resolve needs a vector
// per pixel, not per 2x2 quad. This is the cheap half of the
// reconstruction — positions only, no normals, no tangents, no texture
// sampling — so running it at full resolution costs a fraction of what
// shading there would.

struct MotionUniforms {
    // 🔴 Both UNJITTERED, and that is not a naming detail. Sub-pixel
    // jitter is what the temporal resolve accumulates; a motion vector
    // carrying it would describe the jitter as scene motion and the
    // reprojection would cancel exactly the signal TAA exists to
    // integrate.
    clip_from_world: mat4x4<f32>,
    previous_clip_from_world: mat4x4<f32>,
}

// Group 2: the concatenated prefix takes 0 (vbuf, camera, screen), 1
// (the mesh pool) and 3 (visible meshlets + instances).
@group(2) @binding(0) var<storage, read> previous_transforms: array<mat4x4<f32>>;
@group(2) @binding(1) var<uniform> motion: MotionUniforms;

fn corner_previous_position(previous: mat4x4<f32>, global_vertex: u32) -> vec4<f32> {
    let v = vertices[global_vertex];
    let local = vec4<f32>(v.position[0], v.position[1], v.position[2], 1.0);
    return previous * local;
}

/// Bevy's `calculate_motion_vector` (`pbr_prepass_functions.wesl:93`),
/// line for line.
///
/// The result is a UV offset in the range -1..1, so a pixel can point at
/// the diagonally opposite corner in either direction. Clip space spans
/// -2..2 between those corners, hence the 0.5; V runs down where clip Y
/// runs up, hence the flip.
fn calculate_motion_vector(world_position: vec4<f32>, previous_world_position: vec4<f32>) -> vec2<f32> {
    let clip_position_t = motion.clip_from_world * world_position;
    let clip_position = clip_position_t.xy / clip_position_t.w;
    let previous_clip_position_t = motion.previous_clip_from_world * previous_world_position;
    let previous_clip_position = previous_clip_position_t.xy / previous_clip_position_t.w;
    return (clip_position - previous_clip_position) * vec2<f32>(0.5, -0.5);
}

// 🔴 A fragment pass, not a compute one, and the reason is the format.
// `Rg16Float` is not a storage-texture format in WebGPU's core set, so a
// compute shader cannot write it — the alternatives that can are
// `Rgba16Float` and `Rg32Float`, both eight bytes a pixel against four.
// As a render attachment `Rg16Float` is fine, which is also how Bevy's
// prepass writes it. Half the bandwidth of a pass we are adding to a
// frame the device already says is memory-bound.
struct Varyings {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_motion_vectors(@builtin(vertex_index) index: u32) -> Varyings {
    var out: Varyings;
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

@fragment
fn fs_motion_vectors(in: Varyings) -> @location(0) vec2<f32> {
    let pixel = vec2<u32>(in.position.xy);

    // 🔴 The ids are the LOW half. The high 32 bits are the depth key
    // `textureAtomicMax` sorts on, so they are what says "covered" —
    // reading the payload to test coverage would call a pixel empty
    // wherever the winning triangle happened to be slot 0.
    let packed = textureLoad(vbuf64, pixel).x;
    if ((packed >> 32u) == 0lu) {
        // Background. Zero, not "unwritten": a temporal resolve reads
        // every pixel, and whatever the texture happened to contain
        // would reproject the sky from somewhere else.
        return vec2<f32>(0.0);
    }
    let payload = u32(packed);
    let visible_slot = payload >> 7u;
    let tri_idx = payload & 0x7Fu;

    let packed_visible = visible_meshlets[visible_slot];
    let inst_id = packed_visible >> 16u;
    let meshlet_id = packed_visible & 0xffffu;
    let inst = instances[inst_id];
    let desc = descriptors[meshlet_id];

    let g0 = global_vertex_id(desc, tri_idx, 0u);
    let g1 = global_vertex_id(desc, tri_idx, 1u);
    let g2 = global_vertex_id(desc, tri_idx, 2u);

    let wp0 = corner_world_position(inst, g0);
    let wp1 = corner_world_position(inst, g1);
    let wp2 = corner_world_position(inst, g2);

    let frag_coord = in.position.xy;
    let ndc = frag_coord_to_ndc(frag_coord);
    let half_screen = vec2<f32>(screen.size) * 0.5;
    let pd = compute_partial_derivatives(array<vec4<f32>, 3>(wp0, wp1, wp2), ndc, half_screen);

    let world_position = mat3x4<f32>(wp0, wp1, wp2) * pd.barycentrics;

    // 🔴 The SAME barycentrics, against the previous matrix. Recomputing
    // them from the previous positions would ask where the pixel's
    // *screen* position was, which is the question that cannot survive a
    // LOD change.
    let previous = previous_transforms[inst_id];
    let pp0 = corner_previous_position(previous, g0);
    let pp1 = corner_previous_position(previous, g1);
    let pp2 = corner_previous_position(previous, g2);
    let previous_world_position = mat3x4<f32>(pp0, pp1, pp2) * pd.barycentrics;

    return calculate_motion_vector(world_position, previous_world_position);
}
