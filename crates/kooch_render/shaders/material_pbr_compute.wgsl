// material_pbr_compute.wgsl — the compute shading body (#824).
//
// The same shading `material_pbr_default.wgsl` does, moved out of the
// fragment stage so a workgroup can own a screen tile and read that
// tile's light list once instead of once per pixel.
//
// CONCATENATED by the same `compose_material_shader` the fragment body
// uses, so everything below the entry point is shared, line for line:
// `visibility_buffer_resolve.wgsl` (the R64 read, groups 0), the
// barycentric reconstruction, the contact-shadow march, and Inti.
//
// # Why this is legal in a compute stage
//
// The fragment path samples with `textureSampleGrad` and feeds it the
// derivatives the surface reconstruction computes analytically —
// automatic quad derivatives are wrong here anyway, because two
// fragments in the same 2x2 quad can reconstruct from different
// triangles. A call that takes its gradients explicitly needs no
// fragment stage, so this path samples exactly the same three maps.
//
// (The R32 compute path in `meshlet_deferred.wgsl` says the opposite in
// a comment and shades from the material scalars alone. That comment is
// wrong, and #824 is the issue that found it out.)
//
// # What the measurement asked for
//
// `raster + shade` is 31.41 ms on the OneXFly and is **not ALU-bound**:
// deleting the whole specular model bought 10 % (#821). The grid is not
// over-listing either — 14.9 lights mean against 12 that reach a point
// (#820). What was left is that every pixel fetches its froxel's ~15
// `IntiLight` records out of a storage buffer, and every pixel of a tile
// fetches the same ones.

struct MaterialParams {
    base_color: vec4<f32>,
    // x metallic, y roughness, z emissive, w pad.
    metallic_roughness_emissive_pad: vec4<f32>,
    texture_indices: vec4<u32>,
}

// Group 0 holds the vbuf (0), camera (1), screen (2) and the contact
// shadow's UBO (3) + depth (4) — all declared by the concatenated
// prefix. The shading target is this path's own, and the first free
// index.
@group(0) @binding(5) var color_out: texture_storage_2d<rgba8unorm, write>;

@group(2) @binding(0) var<storage, read> materials: array<MaterialParams>;

@group(4) @binding(0) var albedo_tex: texture_2d<f32>;
@group(4) @binding(1) var normal_tex: texture_2d<f32>;
@group(4) @binding(2) var metal_rough_tex: texture_2d<f32>;
@group(4) @binding(3) var material_sampler: sampler;

// Tile edge in pixels. 16x16 = 256 threads, one wavefront's worth of
// work per lane on AMD at wave32 and the size every tiled-deferred
// reference lands on. It is also small enough that a tile usually sits
// inside one froxel column: at 1280x720 the grid's cells are ~75x80 px.
const TILE_SIZE: u32 = 16u;
const TILE_THREADS: u32 = TILE_SIZE * TILE_SIZE;

// 🔴 A CAP, and what happens when it is hit.
//
// A tile's pixels do not all sit in one froxel. They share a column of
// the grid almost always (the cells are wider than the tile), but depth
// varies across a tile — and at a silhouette it varies a lot, because
// the near surface and whatever is behind it land many z-slices apart.
// So the tile draws its lights from a BLOCK of cells, `[min..max]` per
// axis, and the block is only small because real tiles are mostly one
// continuous surface.
//
// When a tile's block exceeds either cap, the tile shades from the
// storage buffer exactly as the fragment path does. That is slower and
// it is correct — which is the right way around, because a tile that
// straddles a silhouette is rare and a tile that renders wrong is not
// something a capture would ever show us.
const MAX_TILE_CELLS: u32 = 16u;
const MAX_TILE_LIGHTS: u32 = 384u;

// The block of froxels this tile covers, reduced from its own threads.
// Held per axis: a linear cell index cannot be reduced this way, because
// two pixels one slice apart differ by 1 in z and by an arbitrary amount
// in the linear index.
var<workgroup> tile_cell_min: array<atomic<u32>, 3>;
var<workgroup> tile_cell_max: array<atomic<u32>, 3>;

// The tile's light indices, one contiguous run per cell of the block.
var<workgroup> tile_lights: array<u32, MAX_TILE_LIGHTS>;
var<workgroup> tile_cell_start: array<u32, MAX_TILE_CELLS>;
var<workgroup> tile_cell_len: array<u32, MAX_TILE_CELLS>;

// Set when the block did not fit. Read by every thread after the load
// barrier; written only by lane 0, before it.
var<workgroup> tile_overflow: u32;

/// Inti's shading, with the punctual lights taken from workgroup memory
/// instead of the cluster index buffer.
///
/// 🔴 Same lights, same order, same arithmetic as `inti_shade`. The only
/// difference is where `inti_cluster_indices` was read from — which is
/// the entire point of the issue, and the reason the image must not
/// move by one bit.
fn shade_from_tile(
    world_position: vec3<f32>,
    n: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    frag_coord: vec2<f32>,
    flags: u32,
    start: u32,
    len: u32,
) -> vec3<f32> {
    let surf = inti_surface(world_position, n, base_color, metallic, roughness, flags);

    var radiance = vec3<f32>(0.0);
    // Directional lights are not in the grid — they reach every cell, so
    // a cell listing them would say nothing. They are the light buffer's
    // leading entries and this walk is unchanged.
    for (var i = 0u; i < inti.directional_count; i = i + 1u) {
        radiance += inti_light_contribution(surf, inti_lights[i], frag_coord);
    }
    // `KOOCH_LIGHT_LIMIT`, the same cap `inti_clustered_lights` applies
    // to the storage walk. Both paths have to honour it or an A/B
    // between them stops being one.
    var walk = len;
    if (inti.light_limit != 0u) {
        walk = min(walk, inti.light_limit);
    }
    for (var i = 0u; i < walk; i = i + 1u) {
        radiance += inti_light_contribution(
            surf, inti_lights[tile_lights[start + i]], frag_coord);
    }
    radiance += inti_ambient(n, surf.diffuse_color, surf.f0, surf.f_ab);
    return radiance;
}

@compute @workgroup_size(16, 16, 1)
fn cs_shade_tile(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    // 🔴 NO EARLY RETURN ANYWHERE ABOVE THE LAST BARRIER. A thread that
    // leaves the function skips the `workgroupBarrier`s its neighbours
    // are still waiting on, which is undefined behaviour and in practice
    // a hang. Pixels outside the screen, pixels of another material and
    // background pixels all walk the whole function and simply write
    // nothing.
    if (lid < 3u) {
        atomicStore(&tile_cell_min[lid], 0xffffffffu);
        atomicStore(&tile_cell_max[lid], 0u);
    }
    if (lid == 0u) {
        tile_overflow = 0u;
    }
    workgroupBarrier();

    let pixel = gid.xy;
    let inside = pixel.x < screen.size.x && pixel.y < screen.size.y;
    // The pixel centre — the same coordinate the fragment path's
    // `@builtin(position)` carries, so the froxel lookup and the
    // contact-shadow dither agree between the two paths.
    let frag_coord = vec2<f32>(pixel) + vec2<f32>(0.5);

    // Phase 1 — decode the visibility buffer and claim the pixel.
    //
    // Resolving the material takes two dependent reads (`visible_slot` →
    // instance → `material_id`) and stops there: the full barycentric
    // reconstruction runs only for the pixels this dispatch owns, so a
    // scene with four materials does not reconstruct every pixel four
    // times.
    var mine = false;
    var surf: VertexOutput;
    var my_cell = vec3<u32>(0u);
    if (inside) {
        let visibility = textureLoad(vbuf64, pixel).x;
        // `packed >> 32 == 0` is the background sentinel under
        // reversed-Z, the same test `resolve_material_depth.wgsl` makes
        // before it discards.
        if ((visibility >> 32u) != 0lu) {
            let visible_slot = u32(visibility) >> 7u;
            let inst_id = visible_meshlets[visible_slot] >> 16u;
            if (instances[inst_id].material_id == screen.material_id) {
                mine = true;
                surf = resolve_surface(visible_slot, u32(visibility) & 0x7Fu, frag_coord);
                my_cell = inti_cluster_cell(surf.world_position, frag_coord);
                atomicMin(&tile_cell_min[0], my_cell.x);
                atomicMin(&tile_cell_min[1], my_cell.y);
                atomicMin(&tile_cell_min[2], my_cell.z);
                atomicMax(&tile_cell_max[0], my_cell.x);
                atomicMax(&tile_cell_max[1], my_cell.y);
                atomicMax(&tile_cell_max[2], my_cell.z);
            }
        }
    }
    workgroupBarrier();

    // Phase 2 — cache the block's light lists.
    //
    // Every thread computes the same bounds off the same atomics, so
    // `cursor` below advances identically in all of them and the runs
    // need no atomic to be laid out. The loop bound is uniform across
    // the workgroup, which is what lets the barrier after it be reached
    // by everyone.
    let lo = vec3<u32>(
        atomicLoad(&tile_cell_min[0]),
        atomicLoad(&tile_cell_min[1]),
        atomicLoad(&tile_cell_min[2]));
    let hi = vec3<u32>(
        atomicLoad(&tile_cell_max[0]),
        atomicLoad(&tile_cell_max[1]),
        atomicLoad(&tile_cell_max[2]));
    // `lo.x > hi.x` means no thread claimed a pixel: the tile is all
    // background, all another material, or off screen.
    let empty = lo.x > hi.x;
    var dims = vec3<u32>(1u);
    var cell_count = 0u;
    if (!empty) {
        dims = hi - lo + vec3<u32>(1u);
        cell_count = dims.x * dims.y * dims.z;
    }
    let clustered = inti.clustered != 0u;
    // Cell count is checked before the loop; the light total is checked
    // inside it, because it is not known until the records are read.
    var cursor = 0u;
    if (clustered && cell_count > 0u && cell_count <= MAX_TILE_CELLS) {
        for (var c = 0u; c < cell_count; c = c + 1u) {
            let cell = lo + vec3<u32>(
                c / (dims.y * dims.z),
                (c / dims.z) % dims.y,
                c % dims.z);
            let rec = inti_clusters[inti_cluster_index(cell)];
            // Points and spots are consecutive ranges of the same
            // record, walked as one run — the grid stores them that way
            // precisely so no type test has to exist in the loop.
            //
            // Truncating against `cluster_capacity` reproduces the
            // fragment path's `break`: a frame whose lighting overflowed
            // the index list leaves later cells pointing past its end,
            // and both paths must stop at the same light or the images
            // differ where it matters least and is hardest to see.
            let end = min(rec.offset + rec.point_count + rec.spot_count, inti.cluster_capacity);
            let len = select(0u, end - rec.offset, end > rec.offset);
            if (cursor + len > MAX_TILE_LIGHTS) {
                if (lid == 0u) {
                    tile_overflow = 1u;
                }
                break;
            }
            if (lid == 0u) {
                tile_cell_start[c] = cursor;
                tile_cell_len[c] = len;
            }
            for (var i = lid; i < len; i = i + TILE_THREADS) {
                tile_lights[cursor + i] = inti_cluster_indices[rec.offset + i];
            }
            cursor = cursor + len;
        }
    } else if (!empty) {
        if (lid == 0u) {
            tile_overflow = 1u;
        }
    }
    workgroupBarrier();

    // Phase 3 — shade.
    if (mine) {
        let mat = materials[screen.material_id];
        let albedo = textureSampleGrad(
            albedo_tex, material_sampler, surf.uv, surf.ddx_uv, surf.ddy_uv);
        let base = albedo.rgb * mat.base_color.rgb;

        let n_ts = textureSampleGrad(
            normal_tex, material_sampler, surf.uv, surf.ddx_uv, surf.ddy_uv).xyz * 2.0 - 1.0;
        let n = normalize(surf.world_normal);
        let t = normalize(surf.world_tangent.xyz);
        let b = cross(n, t) * surf.world_tangent.w;
        let world_n = normalize(mat3x3<f32>(t, b, n) * n_ts);

        var rgb: vec3<f32>;
        // The debug views (#743). `inti_debug_is_view` is a literal
        // `false` in a production pipeline, so this branch and every
        // view behind it are gone before register allocation.
        if (inti_debug_is_view(screen.debug_mode)) {
            rgb = inti_debug_view(screen.debug_mode, surf.world_position, world_n, frag_coord);
        } else {
            let mr = textureSampleGrad(
                metal_rough_tex, material_sampler, surf.uv, surf.ddx_uv, surf.ddy_uv);
            let metallic = mat.metallic_roughness_emissive_pad.x * mr.b;
            let roughness = mat.metallic_roughness_emissive_pad.y * mr.g;

            var radiance: vec3<f32>;
            if (tile_overflow == 0u && clustered) {
                let my = my_cell - lo;
                let c = (my.x * dims.y + my.y) * dims.z + my.z;
                radiance = shade_from_tile(
                    surf.world_position, world_n, base, metallic, roughness,
                    frag_coord, surf.flags, tile_cell_start[c], tile_cell_len[c]);
            } else {
                // The fallback the caps promise: straight to the storage
                // buffer, the same call the fragment path makes.
                radiance = inti_shade(
                    surf.world_position, world_n, base, metallic, roughness,
                    frag_coord, surf.flags);
            }
            radiance += base * mat.metallic_roughness_emissive_pad.z;
            rgb = inti_tonemap(radiance);
        }
        textureStore(color_out, vec2<i32>(pixel), vec4<f32>(rgb, 1.0));
    }
}
