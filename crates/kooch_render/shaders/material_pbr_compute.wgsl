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
    // xy tiling, zw offset. See `MaterialParams` in `material/mod.rs`:
    // this struct is declared here and in two other shaders, and a test
    // reads all three because a field added to two of them fails
    // silently rather than at compile time.
    uv_scale_offset: vec4<f32>,
}

// Group 0 holds the vbuf (0), camera (1), screen (2) and the contact
// shadow's UBO (3) + depth (4) — all declared by the concatenated
// prefix. The shading targets are this path's own, and the first free
// indices.
//
// At `screen.shading_rate == 1` `color_out` is the HDR target the
// tonemap pass resolves; at 2 it is a half-resolution one the upsample
// pass reads back first (#825).
//
// 🔴 `rgba16float`, and it holds LINEAR RADIANCE — not a picture (#732).
// The format has to match `HDR_COLOR_FORMAT` exactly: wgpu compares the
// storage class declared here against the bind group layout and rejects
// the pipeline, which surfaces as "Texture class Storage doesn't match
// the shader" and not as a wrong image.
@group(0) @binding(5) var color_out: texture_storage_2d<rgba16float, write>;

// #825 — which surface each shaded sample came from, as
// `visible_slot + 1` (0 means the sample shaded nothing). The upsample
// pass compares it against the full-resolution vbuf to decide which
// samples a pixel is allowed to blend, which is what keeps silhouettes
// sharp when the lighting is not. Written only at half rate.
@group(0) @binding(6) var shaded_ids: texture_storage_2d<r32uint, write>;

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

    // One march per pixel rather than one per light (#845), exactly as
    // `inti_shade` does it — the two walks have to agree or the A/B
    // between the paths stops being one.
    var acc = IntiAccum(vec3<f32>(0.0), vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
    let dominant = inti_contact_dominant_only();
    // Directional lights are not in the grid — they reach every cell, so
    // a cell listing them would say nothing. They are the light buffer's
    // leading entries and this walk is unchanged.
    for (var i = 0u; i < inti.directional_count; i = i + 1u) {
        acc = inti_accumulate(acc, inti_light_lit(
            surf, inti_lights[i], i, frag_coord, !dominant));
    }
    // `KOOCH_LIGHT_LIMIT`, the same cap `inti_clustered_lights` applies
    // to the storage walk. Both paths have to honour it or an A/B
    // between them stops being one.
    var walk = len;
    if (inti.light_limit != 0u) {
        walk = min(walk, inti.light_limit);
    }
    for (var i = 0u; i < walk; i = i + 1u) {
        acc = inti_accumulate(acc, inti_light_lit(
            surf, inti_lights[tile_lights[start + i]], tile_lights[start + i], frag_coord, !dominant));
    }
    var radiance = acc.radiance;
    if (dominant && acc.reach > 0.0) {
        let shadow = inti_contact_shadow(
            surf.world_position, surf.n, surf.v, acc.to_light, frag_coord);
        radiance -= acc.brightest * (1.0 - shadow);
    }
    radiance += inti_ambient(n, surf.diffuse_color, surf.f0, surf.f_ab);
    return radiance;
}

// `MeshletDebugMode::TextureMipLevel`, pinned by a test in `debug.rs`.
const DEBUG_TEXTURE_MIP_LEVEL: u32 = 18u;

// The mip level this pixel would sample, computed the way the hardware
// computes it: the uv footprint in texels, log2 of the longer axis.
//
// 🔴 WGSL has no `textureQueryLod`, so this is the formula rather than
// the driver's answer — but it is the SAME formula
// `textureSampleGrad` applies to the same two derivatives, which is
// what makes it worth painting. If this says 10 on a surface filling
// the screen, the sampler is being asked for 10.
fn debug_mip_level(dims: vec2<f32>, ddx: vec2<f32>, ddy: vec2<f32>) -> f32 {
    let footprint = max(length(ddx * dims), length(ddy * dims));
    return max(0.0, log2(max(footprint, 1e-6)));
}

// One colour per whole level, so the frame reads as bands rather than as
// a gradient — a band that moves with the camera is a LOD that works,
// and a screen of one colour is the fault this view exists to show.
// Blue is level 0, and it warms as the level climbs.
fn debug_mip_colour(lod: f32) -> vec3<f32> {
    let level = floor(lod);
    let ramp = clamp(level / 10.0, 0.0, 1.0);
    let base = vec3<f32>(ramp, 1.0 - abs(ramp - 0.5) * 2.0, 1.0 - ramp);
    // The fractional part darkens within a band, so the boundary between
    // two levels is a visible step and not a guess.
    return base * (0.55 + 0.45 * fract(lod));
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

    // The sample this thread owns, and the quad of pixels it stands for.
    // At full rate the quad is one pixel and `pixel == sample`.
    let sample = gid.xy;
    let rate = screen.shading_rate;
    let origin = sample * rate;

    // 🔴 The representative is chosen from the VISIBILITY BUFFER ALONE —
    // the first covered pixel of the quad, in a fixed order — and never
    // from this dispatch's material.
    //
    // Every material dispatch runs over every sample, so if each picked
    // the pixel of its own material the two would write the same texel
    // and the last one to run would win. Choosing from the vbuf makes
    // the representative a property of the frame: exactly one dispatch
    // finds `material_id` equal, exactly one writes.
    //
    // It also removes a hole the upsample would otherwise have. A
    // covered pixel's own quad always contains a covered pixel — itself
    // — so its own quad's sample was shaded, and that sample is always
    // one of the four the upsample considers. There is no covered pixel
    // anywhere on screen with nothing to read from.
    var pixel = origin;
    var visibility = 0lu;
    var covered = false;
    let quad = rate * rate;
    for (var q = 0u; q < quad; q = q + 1u) {
        let cand = origin + vec2<u32>(q % rate, q / rate);
        if (cand.x < screen.size.x && cand.y < screen.size.y) {
            let packed = textureLoad(vbuf64, cand).x;
            // `packed >> 32 == 0` is the background sentinel under
            // reversed-Z, the same test `resolve_material_depth.wgsl`
            // makes before it discards.
            if ((packed >> 32u) != 0lu) {
                pixel = cand;
                visibility = packed;
                covered = true;
                break;
            }
        }
    }
    // The pixel centre — the same coordinate the fragment path's
    // `@builtin(position)` carries, so the froxel lookup and the
    // contact-shadow dither agree between the two paths.
    let frag_coord = vec2<f32>(pixel) + vec2<f32>(0.5);

    // Phase 1 — decode the visibility buffer and claim the sample.
    //
    // Resolving the material takes two dependent reads (`visible_slot` →
    // instance → `material_id`) and stops there: the full barycentric
    // reconstruction runs only for the pixels this dispatch owns, so a
    // scene with four materials does not reconstruct every pixel four
    // times.
    var mine = false;
    var surf: VertexOutput;
    var my_cell = vec3<u32>(0u);
    var my_slot = 0u;
    if (covered) {
        let visible_slot = u32(visibility) >> 7u;
        let inst_id = visible_meshlets[visible_slot] >> 16u;
        if (instances[inst_id].material_id == screen.material_id) {
            mine = true;
            my_slot = visible_slot;
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
                let light_index = inti_cluster_indices[rec.offset + i];
                tile_lights[cursor + i] = light_index;
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

        // 🔴 The DERIVATIVES scale with the coordinate, and forgetting
        // that is the trap. `textureSampleGrad` picks the mip from how
        // fast the uv moves between pixels; tiling a texture twenty
        // times makes it move twenty times faster, and handing the
        // untiled derivatives selects a level about four steps too
        // sharp. The result is the aliasing the mip chain exists to
        // remove, on exactly the surfaces that asked for tiling.
        let uv = surf.uv * mat.uv_scale_offset.xy + mat.uv_scale_offset.zw;
        // 🔴 The mip bias rides on the SAME multiply (#881). A bias is
        // `lod += b`, and `lod` is `log2(footprint)`, so scaling the
        // footprint by `exp2(b)` is the bias exactly — no `log2` per
        // pixel and no sampler feature, which wgpu does not expose
        // anyway.
        let derivative_scale = mat.uv_scale_offset.xy * screen.mip_bias_scale;
        let ddx_uv = surf.ddx_uv * derivative_scale;
        let ddy_uv = surf.ddy_uv * derivative_scale;
        let albedo = textureSampleGrad(
            albedo_tex, material_sampler, uv, ddx_uv, ddy_uv);
        let base = albedo.rgb * mat.base_color.rgb;

        let n_ts = textureSampleGrad(
            normal_tex, material_sampler, uv, ddx_uv, ddy_uv).xyz * 2.0 - 1.0;
        let n = normalize(surf.world_normal);
        let t = normalize(surf.world_tangent.xyz);
        let b = cross(n, t) * surf.world_tangent.w;
        let world_n = normalize(mat3x3<f32>(t, b, n) * n_ts);

        var rgb: vec3<f32>;
        // The debug views (#743). `inti_debug_is_view` is a literal
        // `false` in a production pipeline, so this branch and every
        // view behind it are gone before register allocation.
        //
        // The mip view is resolved HERE rather than inside Inti: it is a
        // question about the material's sampling, and Inti is handed a
        // world position and a normal — it has never seen a uv.
        if (screen.debug_mode == DEBUG_TEXTURE_MIP_LEVEL) {
            let dims = vec2<f32>(textureDimensions(albedo_tex, 0));
            if (dims.x <= 1.0 && dims.y <= 1.0) {
                // The 1x1 fallback: no albedo map, so no chain to pick
                // from and nothing this view can say.
                rgb = vec3<f32>(1.0, 0.0, 1.0);
            } else {
                rgb = debug_mip_colour(debug_mip_level(dims, ddx_uv, ddy_uv));
            }
        } else if (inti_debug_is_view(screen.debug_mode)) {
            rgb = inti_debug_view(screen.debug_mode, surf.world_position, world_n, frag_coord);
        } else {
            let mr = textureSampleGrad(
                metal_rough_tex, material_sampler, uv, ddx_uv, ddy_uv);
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
            // 🔴 Linear radiance out, NOT a picture (#732). The tonemap
            // is its own pass now, because temporal anti-aliasing blends
            // this frame with the last and an average of two
            // ACES-tonemapped values is not the tonemap of their average.
            //
            // The debug branch above is the exception and keeps its own
            // colour: those views produce a legend, and the tonemap pass
            // is switched off for them rather than asked to undo one.
            rgb = radiance;
        }
        textureStore(color_out, vec2<i32>(sample), vec4<f32>(rgb, 1.0));
        // Only the upsample reads this, and only half rate has one. At
        // full rate the store would be one write per pixel bought
        // nothing — the branch is uniform across the workgroup, so it
        // costs a scalar compare.
        if (rate > 1u) {
            textureStore(shaded_ids, vec2<i32>(sample), vec4<u32>(my_slot + 1u, 0u, 0u, 0u));
        }
    }
}
