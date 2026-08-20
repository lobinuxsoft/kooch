// page_mark.wgsl — which shadow pages a frame actually needs (#866).
//
// CONCATENATED after `cluster_common.wgsl`, the way the grid's own
// passes are built, so `ClusterView`, `ClusterCell`, `ClusterLight` and
// the two cell lookups are the grid's declarations rather than a second
// copy free to drift from them.
//
// # What marks a page, and why it is not the grid alone
//
// One thread per screen pixel — or per block of them, see `rate`. The
// depth buffer says WHERE a surface is; the froxel grid says WHICH
// lights reach it. Both are needed and neither is sufficient:
//
// - Marking from the grid's cells alone claims pages for ground no
//   surface occupies. Measured on `many_lights.scene`, that is 15770
//   pages for the sun where the surfaces need 118 — 133x.
// - Marking from depth alone would have to walk every light per pixel,
//   which is the loop the grid exists to remove.
//
// Epic states the first half in one sentence — "depth buffer analysis is
// used as the primary method of marking pages that are needed to render"
// — and the Chalmers papers state the second: the page allocation is
// driven by the cluster's view samples.
//
// # The mirror
//
// Every arithmetic decision below has a twin in `shadow/pages.rs` on the
// CPU: `page_level`/`level_for`, `sun_level`/`level_above`+`level_below`,
// `page_of`/`page_rect`. That is deliberate — the CPU census is this
// pass's oracle, and two counts that disagree mean one of them is wrong.

// Mirrors `PageMarkView` in `pages/mark.rs`, field for field.
struct PageView {
    world_from_clip: mat4x4<f32>,
    // xyz the camera, w the clipmap's level-0 extent in metres.
    eye_and_base: vec4<f32>,
    // xyz the sun's direction, w 1 if there is one.
    sun: vec4<f32>,
    // x page texels, y virtual texels, z levels in a local chain,
    // w levels in the clipmap.
    chain: vec4<u32>,
    // x pages per side at level 0, y pages in one face's whole chain,
    // z the per-light stride in pages, w the light count.
    strides: vec4<u32>,
    // x the sampling rate in pixels, y the sun's slot, zw unused.
    sampling: vec4<u32>,
}

@group(0) @binding(0) var<uniform> view: ClusterView;
// The read side of `ClusterCell`, without the atomics.
//
// A read-only storage binding cannot hold `atomic<u32>`, and the grid
// declares its cells atomic because two rasterizer passes write them
// from many fragments at once. `inti_pbr.wgsl` mirrors the same record
// as `IntiClusterCell` for the same reason; this is the third reader and
// the layout is pinned by the same test.
struct PageCell {
    offset: u32,
    point_count: u32,
    spot_count: u32,
    probe_count: u32,
    volume_count: u32,
    decal_count: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(1) var<storage, read> cells: array<PageCell>;
@group(0) @binding(2) var<storage, read> indices: array<u32>;
@group(0) @binding(3) var<storage, read> lights: array<ClusterLight>;
@group(0) @binding(4) var depth_tex: texture_depth_2d;
@group(0) @binding(5) var<uniform> pages: PageView;
@group(0) @binding(6) var<storage, read_write> marks: array<atomic<u32>>;
// x the distinct pages marked, y the samples that found a surface,
// z pairs visited, w overflow — a page index past the buffer.
@group(0) @binding(7) var<storage, read_write> counters: array<atomic<u32>>;

const MARK_GROUP: u32 = 8u;

// Mirrors `gpu_light.rs`. Spelled out because the census's twin in Rust
// reads the same constants from that file.
const LIGHT_KIND_SPOT: u32 = 2u;

// One bit, set once. The return says whether this thread is the one that
// set it, which is what makes the counter a count of DISTINCT pages
// rather than of marking attempts.
fn mark_bit(index: u32) -> bool {
    let word = index / 32u;
    if word >= arrayLength(&marks) {
        atomicAdd(&counters[3], 1u);
        return false;
    }
    let bit = 1u << (index % 32u);
    let was = atomicOr(&marks[word], bit);
    if (was & bit) != 0u {
        return false;
    }
    atomicAdd(&counters[0], 1u);
    return true;
}

// Where `level` starts inside one face's chain. Mirrors
// `PageConfig::level_base`: a mip chain's levels are not the same size,
// so the offset is a running sum and not a multiply.
fn level_base(level: u32) -> u32 {
    var base = 0u;
    var side = pages.strides.x;
    for (var l = 0u; l < level; l = l + 1u) {
        base = base + side * side;
        side = max(side / 2u, 1u);
    }
    return base;
}

fn level_side(level: u32) -> u32 {
    return max(pages.strides.x >> level, 1u);
}

// The coarsest level whose texels are still at least as dense as the
// screen's pixels. A cube face spans 90 degrees, so at `distance` it
// covers `2 * distance` world units across its texels.
fn page_level(distance: f32, wanted: f32) -> u32 {
    if wanted <= 0.0 {
        return 0u;
    }
    let texels = 2.0 * distance / wanted;
    if texels <= 0.0 {
        return pages.chain.z - 1u;
    }
    let level = floor(log2(f32(pages.chain.y) / texels));
    return min(u32(max(level, 0.0)), pages.chain.z - 1u);
}

// Which of the six cube faces a direction lands on, and its position
// across that face. Mirrors what `face_view_proj` produces without
// building the matrix: the major axis picks the face, and the other two
// divided by it are the face's normalised coordinates.
fn cube_face(dir: vec3<f32>) -> vec4<f32> {
    let a = abs(dir);
    var face = 0u;
    var uv = vec2<f32>(0.0);
    var major = 0.0;
    if a.x >= a.y && a.x >= a.z {
        major = a.x;
        face = select(1u, 0u, dir.x > 0.0);
        uv = select(vec2<f32>(dir.z, -dir.y), vec2<f32>(-dir.z, -dir.y), dir.x > 0.0);
    } else if a.y >= a.z {
        major = a.y;
        face = select(3u, 2u, dir.y > 0.0);
        uv = select(vec2<f32>(dir.x, -dir.z), vec2<f32>(dir.x, dir.z), dir.y > 0.0);
    } else {
        major = a.z;
        face = select(5u, 4u, dir.z < 0.0);
        uv = select(vec2<f32>(dir.x, -dir.y), vec2<f32>(-dir.x, -dir.y), dir.z < 0.0);
    }
    if major <= 0.0 {
        return vec4<f32>(0.5, 0.5, 0.0, f32(face));
    }
    return vec4<f32>(uv / major * 0.5 + vec2<f32>(0.5), 0.0, f32(face));
}

// One page of a local light's mip chain.
fn mark_local(light: u32, world: vec3<f32>, wanted: f32) {
    let record = lights[light];
    let offset = world - record.position;
    let distance = max(length(offset), 0.05);
    let level = page_level(distance, wanted);
    let side = level_side(level);

    let hit = cube_face(offset);
    // A spot writes one face, like `CensusKind::Spot`. `kind` mirrors
    // `GpuLight::kind`, and the order there is DIRECTIONAL 0, POINT 1,
    // SPOT 2 — not the order a reader guesses.
    let face = select(u32(hit.w), 0u, record.kind == LIGHT_KIND_SPOT);
    let cell = vec2<u32>(clamp(hit.xy, vec2<f32>(0.0), vec2<f32>(0.99999)) * f32(side));

    let index = light * pages.strides.z
        + face * pages.strides.y
        + level_base(level)
        + cell.y * side
        + cell.x;
    mark_bit(index);
}

// One page of the sun's clipmap.
//
// Every level is a full grid rather than half of the last — that is what
// a clipmap is and what a mip chain is not — so the offset is a multiply
// where `mark_local`'s is a running sum.
fn mark_sun(slot: u32, world: vec3<f32>, wanted: f32) {
    let direction = normalize(pages.sun.xyz);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if abs(direction.y) > 0.99 {
        up = vec3<f32>(0.0, 0.0, 1.0);
    }
    // The light's basis, built rather than uploaded: the sun has no
    // position, so this is the only place it means anything.
    let f = direction;
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    let offset = world - pages.eye_and_base.xyz;
    let plane = vec2<f32>(dot(offset, s), dot(offset, u));

    let base = pages.eye_and_base.w;
    let texels = f32(pages.chain.y);
    let reach = max(abs(plane.x), abs(plane.y)) * 2.0;
    // Containment is a ceiling on how far the sample is, density a floor
    // on how fine the level may be. Mirrors `mark_sun_cell`.
    let contain = select(0.0, ceil(log2(max(reach / base, 1.0))), reach > base);
    let density = select(0.0, floor(log2(max(wanted * texels / base, 1.0))), wanted * texels > base);
    let level = min(u32(max(contain, density)), pages.chain.w - 1u);

    let extent = base * exp2(f32(level));
    let side = pages.strides.x;
    let uv = clamp(plane / extent + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(0.99999));
    let cell = vec2<u32>(uv * f32(side));

    let index = slot * pages.strides.z + level * side * side + cell.y * side + cell.x;
    mark_bit(index);
}

@compute @workgroup_size(MARK_GROUP, MARK_GROUP, 1)
fn mark_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let rate = max(pages.sampling.x, 1u);
    let pixel = id.xy * rate;
    let size = vec2<u32>(view.viewport.xy);
    if pixel.x >= size.x || pixel.y >= size.y {
        return;
    }

    // 🔴 Reversed-Z infinite (ADR 0002): the buffer clears to 0 and that
    // is the FAR value, so a zero is sky rather than a surface at the
    // near plane. Marking it would put a page under every pixel the
    // scene does not cover, which is the whole failure this pass exists
    // to avoid.
    let depth = textureLoad(depth_tex, vec2<i32>(pixel), 0);
    if depth <= 0.0 {
        return;
    }
    atomicAdd(&counters[1], 1u);

    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / view.viewport.xy;
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let hom = pages.world_from_clip * vec4<f32>(ndc, 1.0);
    if abs(hom.w) < 1e-9 {
        return;
    }
    let world = hom.xyz / hom.w;

    // World metres one screen pixel covers here, from the frustum rather
    // than from the froxel: `2 * d * tan(fov/2) / height`, with the
    // focal length read off the projection. Mirrors
    // `CensusCamera::pixel_at`.
    let view_pos = view.view_from_world * vec4<f32>(world, 1.0);
    let focal = view.clip_from_view[1][1];
    var wanted = 0.0;
    if abs(focal) > 1e-9 {
        wanted = 2.0 * abs(view_pos.z) / (focal * max(view.viewport.y, 1.0));
    }
    // A sample is `rate` pixels wide when the pass runs coarse, and the
    // page it needs has to cover all of them.
    wanted = wanted * f32(rate);

    if pages.sun.w > 0.5 {
        mark_sun(pages.sampling.y, world, wanted);
    }

    if view.dimensions.w == 0u {
        return;
    }
    let cell = cluster_of_ndc(view, ndc, view_pos.z);
    let record = cells[cluster_index(cell, view.dimensions)];
    let start = record.offset;
    // Points and spots are the first two ranges, stored in that order,
    // and both need pages. Probes, volumes and decals do not.
    let count = record.point_count + record.spot_count;
    for (var i = 0u; i < count; i = i + 1u) {
        let slot = start + i;
        if slot >= arrayLength(&indices) {
            break;
        }
        let light = indices[slot];
        if light >= pages.strides.w {
            continue;
        }
        atomicAdd(&counters[2], 1u);
        mark_local(light, world, wanted);
    }
}
