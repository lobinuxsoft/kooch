// The froxel grid's shared declarations (#780).
//
// Concatenated ahead of each clustering pass, which is how WGSL modules
// share anything here — the same mechanism `inti_pbr_shader` uses.
//
// # The shape of the thing
//
// The view frustum is diced into a WxHxD grid of cells ("froxels"),
// logarithmic along the view axis. Each cell holds the indices of the
// lights that reach it. Shading then iterates the lights of ONE cell
// instead of every light in the scene, which is the whole point: the
// cost stops being pixels x lights.
//
// 🔴 The grid is not a light structure. The per-cell record below
// reserves counts for reflection probes, irradiance volumes and decals
// as well, none of which exist in the engine yet. Bevy's does the same,
// and the warning in #780 is explicit: building this as "a list of
// lights per cluster" produces half of it and the other half is a
// rewrite. The five ranges cost five words per cell and nothing per
// pixel — a shader that has no decals never walks that range.

// Values for `ZSlice.object_type`. Ordered: the per-cell lists are
// stored in this order, so a range is a pair of offsets and never a
// per-element type test inside the shading loop.
const CLUSTER_TYPE_POINT: u32 = 0u;
const CLUSTER_TYPE_SPOT: u32 = 1u;
const CLUSTER_TYPE_PROBE: u32 = 2u;
const CLUSTER_TYPE_VOLUME: u32 = 3u;
const CLUSTER_TYPE_DECAL: u32 = 4u;

// Mirrors `GpuLight` in `gpu_light.rs`, field for field. Declared again
// here rather than shared with `inti_pbr.wgsl` because these passes are
// separate modules — and the test that pins the size covers both.
struct ClusterLight {
    color: vec3<f32>,
    intensity: f32,
    position: vec3<f32>,
    range: f32,
    direction: vec3<f32>,
    kind: u32,
    spot_scale: f32,
    spot_offset: f32,
    flags: u32,
    shadow_slot: u32,
    radius: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

// Everything the passes need to know about the view and the grid.
struct ClusterView {
    view_from_world: mat4x4<f32>,
    clip_from_view: mat4x4<f32>,
    view_from_clip: mat4x4<f32>,
    // The reciprocal of the camera's world scale, per axis. A world-space
    // radius is a view-space radius times this — which is 1 for every
    // camera nobody scaled, and not for the ones somebody did.
    view_scale: vec4<f32>,
    // xyz = grid dimensions, w = their product.
    dimensions: vec4<u32>,
    // xy = the logarithmic z-slice constants, zw = the grid's near and
    // far in metres.
    z_factors: vec4<f32>,
    // xy = viewport size in pixels, zw = one cell's size in pixels.
    viewport: vec4<f32>,
    // x = lights in the buffer, y = z-slice list capacity,
    // z = index list capacity, w = unused.
    counts: vec4<u32>,
}

// One entry of the work list the z-slice pass produces and the
// rasterizer consumes: "object O appears in slice Z".
struct ZSlice {
    object_index: u32,
    object_type: u32,
    z_slice: u32,
}

// Per-cell offsets and counts, one per froxel.
//
// `offset` is where this cell's indices start in the shared index list;
// the five counts are the lengths of the five type ranges, in the order
// the constants above declare. Atomic because both rasterizer passes
// write it from many fragments at once.
struct ClusterCell {
    offset: atomic<u32>,
    point_count: atomic<u32>,
    spot_count: atomic<u32>,
    probe_count: atomic<u32>,
    volume_count: atomic<u32>,
    decal_count: atomic<u32>,
    _pad0: u32,
    _pad1: u32,
}

// The draw arguments the rasterizer is dispatched from, plus what the
// CPU reads to size the buffers.
//
// The first four words are exactly `wgpu::util::DrawIndirectArgs`, at
// offset zero, because that is what `draw_indirect` reads.
//
// 🔴 `wanted` counts every (object, slice) pair the grid found, past
// capacity and all, while `instance_count` is that number clamped to
// what the list can hold. Two words instead of one because they answer
// different questions: the draw must not be told to read entries that
// were never written, and the CPU must not be told the frame fit when it
// did not. `cluster_finalize` is what turns one into the other.
struct ClusterDraw {
    vertex_count: u32,
    // Written by `cluster_finalize`, read by `draw_indirect`.
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
    // (object, slice) pairs found this frame, uncapped.
    wanted: atomic<u32>,
    // Index-list entries the grid needs, uncapped, written by the
    // allocation pass. Past the list's capacity means cells were
    // truncated and the buffer has to grow.
    index_size: u32,
}

struct ClusterAabb {
    min: vec3<f32>,
    max: vec3<f32>,
}

// The slice a view-space depth falls in.
//
// `view_z` is negative in front of the camera, hence the negation before
// the logarithm. Mirrored by `ClusterGrid::z_slice` in `grid.rs`, and by
// `inti_cluster_index` in `inti_pbr.wgsl`: three copies of four
// operations, because the alternative is a fragment reading a cell the
// grid never wrote.
fn cluster_z_slice(z_factors: vec2<f32>, z_slices: u32, view_z: f32) -> u32 {
    let slice = log(-view_z) * z_factors.x - z_factors.y + 1.0;
    return min(u32(max(slice, 0.0)), z_slices - 1u);
}

// Where a cell's record lives, given its grid coordinate.
fn cluster_index(p: vec3<u32>, dimensions: vec4<u32>) -> u32 {
    // Clamped rather than trusted: an out-of-range index into a storage
    // array is undefined behaviour, and every caller here derives `p`
    // from interpolated floats.
    return min((p.y * dimensions.x + p.x) * dimensions.z + p.z, dimensions.w - 1u);
}

// The grid coordinate a normalised-device position falls in.
fn cluster_of_ndc(view: ClusterView, ndc: vec3<f32>, view_z: f32) -> vec3<u32> {
    let uv = clamp(ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(1.0));
    let xy = vec2<u32>(floor(uv * vec2<f32>(view.dimensions.xy)));
    let z = cluster_z_slice(view.z_factors.xy, view.dimensions.z, view_z);
    return clamp(vec3<u32>(xy, z), vec3<u32>(0u), view.dimensions.xyz - vec3<u32>(1u));
}

// A sphere's bounds in NDC, with the unprojected view-space z riding in
// the third component.
//
// The four corners are projected at BOTH the near and the far end of the
// sphere: under perspective the point at max z and min xy can land
// further left on screen than the one at min z, so projecting one corner
// pair would miss cells the sphere really covers.
fn cluster_sphere_ndc(
    view: ClusterView,
    position: vec3<f32>,
    radius: f32,
) -> ClusterAabb {
    let center = (view.view_from_world * vec4<f32>(position, 1.0)).xyz;
    let half = radius * abs(view.view_scale.xyz);

    var view_min = center - half;
    var view_max = center + half;
    // Held in front of the camera. At view z = 0 the projected x and y
    // are undefined, and behind the camera perspective flips both axes —
    // which silently swaps every min with its max.
    view_min.z = min(view_min.z, -0.00001);
    view_max.z = min(view_max.z, -0.00001);

    let a = view.clip_from_view * vec4<f32>(view_min, 1.0);
    let b = view.clip_from_view * vec4<f32>(vec3<f32>(view_min.xy, view_max.z), 1.0);
    let c = view.clip_from_view * vec4<f32>(vec3<f32>(view_max.xy, view_min.z), 1.0);
    let d = view.clip_from_view * vec4<f32>(view_max, 1.0);

    let ndc_a = a.xyz / a.w;
    let ndc_b = b.xyz / b.w;
    let ndc_c = c.xyz / c.w;
    let ndc_d = d.xyz / d.w;

    let ndc_min = min(min(ndc_a, ndc_b), min(ndc_c, ndc_d));
    let ndc_max = max(max(ndc_a, ndc_b), max(ndc_c, ndc_d));

    return ClusterAabb(
        vec3<f32>(clamp(ndc_min.xy, vec2<f32>(-1.0), vec2<f32>(1.0)), view_min.z),
        vec3<f32>(clamp(ndc_max.xy, vec2<f32>(-1.0), vec2<f32>(1.0)), view_max.z),
    );
}

// The range of cells a sphere can possibly touch, inclusive.
fn cluster_sphere_bounds(
    view: ClusterView,
    position: vec3<f32>,
    radius: f32,
) -> ClusterAabb {
    let ndc = cluster_sphere_ndc(view, position, radius);
    let a = cluster_of_ndc(view, ndc.min, ndc.min.z);
    let b = cluster_of_ndc(view, ndc.max, ndc.max.z);
    return ClusterAabb(vec3<f32>(min(a, b)), vec3<f32>(max(a, b)));
}

// The world-space bounding sphere of a light, as `vec4(centre, radius)`.
//
// Directional lights have neither, and are not clustered: they reach
// every cell, so a grid says nothing about them. The shading loop keeps
// walking them linearly, which is correct and costs one iteration.
fn cluster_light_sphere(light: ClusterLight) -> vec4<f32> {
    return vec4<f32>(light.position, light.range);
}
