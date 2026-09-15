// material_tile_bins.wgsl — the tiles each material covers, so compute shading dispatches a material
// only over those (#1157), the way Nanite bins shading by material. Two entry points, run in order:
// `cs_classify` over the tiles, then `cs_lists` once.
//
// 🔴 No atomics on buffers. RADV does not make them atomic: concurrent `atomicAdd`s on a shared
// counter lost updates and left tile-sized holes. Every write here has exactly one writer.

struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    lod_force_level: i32,
    group_base: u32,
    flags: u32,
    _pad1: u32,
    _pad2: u32,
}

struct BinParams {
    // Full resolution, as `ScreenUniforms::size`.
    size: vec2<u32>,
    // Shaded tiles per axis.
    tiles: vec2<u32>,
    shading_rate: u32,
    // Shading slots this frame; ids at or above are not binned.
    slots: u32,
    // List entries `bins` has room for.
    capacity: u32,
    _pad: u32,
}

const TILE_SIZE: u32 = 16u;
const TILE_THREADS: u32 = 256u;
const MAX_SLOTS: u32 = 256u;
// One bit per slot.
const WORDS: u32 = 8u;
// `bins` layout: [0] overflow, [1] tiles per row, [2..2+MAX_SLOTS] each slot's first entry and
// [2+MAX_SLOTS] the total, then the list of tile indices.
const LIST: u32 = 259u;
// Workgroups per row of an indirect dispatch, so a long list stays under the per-axis limit.
const ROW: u32 = 4096u;

@group(0) @binding(0) var vbuf64: texture_storage_2d<r64uint, read>;
@group(0) @binding(1) var<storage, read> visible_meshlets: array<u32>;
@group(0) @binding(2) var<storage, read> instances: array<MeshInstance>;
@group(0) @binding(3) var<uniform> params: BinParams;
@group(0) @binding(4) var<storage, read_write> tile_bits: array<u32>;
@group(0) @binding(5) var<storage, read_write> bins: array<u32>;
@group(0) @binding(6) var<storage, read_write> args: array<u32>;

// Each thread's material, written only by that thread.
var<workgroup> thread_slot: array<u32, TILE_THREADS>;

// 🔴 The same representative the shading frame picks: the first covered pixel of the quad.
@compute @workgroup_size(16, 16, 1)
fn cs_classify(
    @builtin(workgroup_id) tile_id: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let rate = params.shading_rate;
    let origin = (tile_id.xy * TILE_SIZE + local.xy) * rate;
    var slot = MAX_SLOTS;
    for (var q = 0u; q < rate * rate; q = q + 1u) {
        let cand = origin + vec2<u32>(q % rate, q / rate);
        if (cand.x < params.size.x && cand.y < params.size.y) {
            let packed = textureLoad(vbuf64, cand).x;
            if ((packed >> 32u) != 0lu) {
                let inst_id = visible_meshlets[u32(packed) >> 7u] >> 16u;
                slot = instances[inst_id].material_id;
                break;
            }
        }
    }
    thread_slot[lid] = slot;
    // No early return above: a thread that leaves skips its neighbours' barrier.
    workgroupBarrier();

    if (lid == 0u) {
        var words: array<u32, WORDS>;
        for (var t = 0u; t < TILE_THREADS; t = t + 1u) {
            let s = thread_slot[t];
            if (s < params.slots) {
                words[s >> 5u] = words[s >> 5u] | (1u << (s & 31u));
            }
        }
        let base = (tile_id.y * params.tiles.x + tile_id.x) * WORDS;
        for (var w = 0u; w < WORDS; w = w + 1u) {
            tile_bits[base + w] = words[w];
        }
    }
}

// One invocation: counts, first entries, dispatch arguments, then the lists.
@compute @workgroup_size(1)
fn cs_lists() {
    let tile_count = params.tiles.x * params.tiles.y;
    var counts: array<u32, MAX_SLOTS>;
    for (var tile = 0u; tile < tile_count; tile = tile + 1u) {
        for (var w = 0u; w < WORDS; w = w + 1u) {
            var bits = tile_bits[tile * WORDS + w];
            while (bits != 0u) {
                let s = w * 32u + firstTrailingBit(bits);
                counts[s] = counts[s] + 1u;
                bits = bits & (bits - 1u);
            }
        }
    }

    var first: array<u32, MAX_SLOTS>;
    var total = 0u;
    for (var s = 0u; s < MAX_SLOTS; s = s + 1u) {
        first[s] = total;
        bins[2u + s] = total;
        total = total + counts[s];
    }
    bins[2u + MAX_SLOTS] = total;
    // Past capacity every covered material dispatches over the whole grid, as before binning.
    let overflow = total > params.capacity;
    bins[0] = select(0u, 1u, overflow);
    bins[1] = params.tiles.x;
    for (var s = 0u; s < MAX_SLOTS; s = s + 1u) {
        let count = counts[s];
        var dispatch = vec3<u32>(0u, 1u, 1u);
        if (count > 0u && overflow) {
            dispatch = vec3<u32>(params.tiles, 1u);
        } else if (count > 0u) {
            dispatch = vec3<u32>(min(count, ROW), (count + ROW - 1u) / ROW, 1u);
        }
        args[s * 3u] = dispatch.x;
        args[s * 3u + 1u] = dispatch.y;
        args[s * 3u + 2u] = dispatch.z;
    }
    if (overflow) {
        return;
    }

    for (var tile = 0u; tile < tile_count; tile = tile + 1u) {
        for (var w = 0u; w < WORDS; w = w + 1u) {
            var bits = tile_bits[tile * WORDS + w];
            while (bits != 0u) {
                let s = w * 32u + firstTrailingBit(bits);
                bins[LIST + first[s]] = tile;
                first[s] = first[s] + 1u;
                bits = bits & (bits - 1u);
            }
        }
    }
}
