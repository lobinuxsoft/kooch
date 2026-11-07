// material_tile_bins.wgsl — the tiles each material covers, so compute shading dispatches a material
// only over those (#1157), the way Nanite bins shading by material. Four entry points, run in order:
// `cs_classify` per tile, `cs_count` per chunk of tiles, `cs_scan` once, `cs_scatter` per chunk.
// One thread per slot in the last three: a single thread walking every tile cost 17.7 ms at 1080p (#1857).
//
// 🔴 No atomics on buffers. RADV does not make them atomic: concurrent `atomicAdd`s on a shared
// counter lost updates and left tile-sized holes. Every buffer write here has exactly one writer.

struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    lod_force_level: i32,
    group_base: u32,
    flags: u32,
    // Which layers the instance is in (#1218). `MeshInstance` in `scene.rs`.
    layers: u32,
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
// Tiles per chunk: the serial walk of each slot's thread in `cs_count` and `cs_scatter`.
const CHUNK: u32 = 64u;

@group(0) @binding(0) var vbuf64: texture_storage_2d<r64uint, read>;
@group(0) @binding(1) var<storage, read> visible_meshlets: array<u32>;
@group(0) @binding(2) var<storage, read> instances: array<MeshInstance>;
@group(0) @binding(3) var<uniform> params: BinParams;
@group(0) @binding(4) var<storage, read_write> tile_bits: array<u32>;
@group(0) @binding(5) var<storage, read_write> bins: array<u32>;
@group(0) @binding(6) var<storage, read_write> args: array<u32>;
// `[chunk * MAX_SLOTS + slot]`: the slot's tiles in that chunk, then where they start in its list.
@group(0) @binding(7) var<storage, read_write> chunk_counts: array<u32>;

// The tile's slots, one bit each. Workgroup atomics are shared memory, not the buffers RADV drops.
var<workgroup> tile_words: array<atomic<u32>, WORDS>;

// 🔴 The same representative the shading frame picks: the first covered pixel of the quad.
@compute @workgroup_size(16, 16, 1)
fn cs_classify(
    @builtin(workgroup_id) tile_id: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    if (lid < WORDS) {
        atomicStore(&tile_words[lid], 0u);
    }
    workgroupBarrier();
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
    if (slot < params.slots) {
        atomicOr(&tile_words[slot >> 5u], 1u << (slot & 31u));
    }
    // No early return above: a thread that leaves skips its neighbours' barrier.
    workgroupBarrier();
    if (lid < WORDS) {
        tile_bits[(tile_id.y * params.tiles.x + tile_id.x) * WORDS + lid] = atomicLoad(&tile_words[lid]);
    }
}

fn covers(tile: u32, slot: u32) -> bool {
    return (tile_bits[tile * WORDS + (slot >> 5u)] & (1u << (slot & 31u))) != 0u;
}

// One workgroup per chunk, one thread per slot: how many of the chunk's tiles each slot covers.
@compute @workgroup_size(256, 1, 1)
fn cs_count(@builtin(workgroup_id) chunk: vec3<u32>, @builtin(local_invocation_index) slot: u32) {
    let tile_count = params.tiles.x * params.tiles.y;
    let first = chunk.x * CHUNK;
    let last = min(first + CHUNK, tile_count);
    var count = 0u;
    for (var tile = first; tile < last; tile = tile + 1u) {
        count = count + select(0u, 1u, covers(tile, slot));
    }
    chunk_counts[chunk.x * MAX_SLOTS + slot] = count;
}

var<workgroup> slot_counts: array<u32, MAX_SLOTS>;
// Read back through the workgroup: a barrier does not make thread 0's buffer write visible.
var<workgroup> overflow: u32;

// One workgroup, one thread per slot: each slot's chunks become offsets in its list, then the slots'
// totals become each list's first entry, the dispatch arguments and the overflow flag.
@compute @workgroup_size(256, 1, 1)
fn cs_scan(@builtin(local_invocation_index) slot: u32) {
    let chunks = (params.tiles.x * params.tiles.y + CHUNK - 1u) / CHUNK;
    var running = 0u;
    for (var c = 0u; c < chunks; c = c + 1u) {
        let at = c * MAX_SLOTS + slot;
        let count = chunk_counts[at];
        chunk_counts[at] = running;
        running = running + count;
    }
    slot_counts[slot] = running;
    workgroupBarrier();

    // 256 steps, once a frame: not worth a parallel scan.
    if (slot == 0u) {
        var total = 0u;
        for (var s = 0u; s < MAX_SLOTS; s = s + 1u) {
            bins[2u + s] = total;
            total = total + slot_counts[s];
        }
        bins[2u + MAX_SLOTS] = total;
        // Past capacity every covered material dispatches over the whole grid, as before binning.
        overflow = select(0u, 1u, total > params.capacity);
        bins[0] = overflow;
        bins[1] = params.tiles.x;
    }
    workgroupBarrier();

    let count = slot_counts[slot];
    var dispatch = vec3<u32>(0u, 1u, 1u);
    if (count > 0u && overflow != 0u) {
        dispatch = vec3<u32>(params.tiles, 1u);
    } else if (count > 0u) {
        dispatch = vec3<u32>(min(count, ROW), (count + ROW - 1u) / ROW, 1u);
    }
    args[slot * 3u] = dispatch.x;
    args[slot * 3u + 1u] = dispatch.y;
    args[slot * 3u + 2u] = dispatch.z;
}

// One workgroup per chunk, one thread per slot: the slot's tiles in the chunk, in tile order, at
// the offset `cs_scan` left. Lists come out as a single walk over the tiles would write them.
@compute @workgroup_size(256, 1, 1)
fn cs_scatter(@builtin(workgroup_id) chunk: vec3<u32>, @builtin(local_invocation_index) slot: u32) {
    if (bins[0] != 0u) {
        return;
    }
    let tile_count = params.tiles.x * params.tiles.y;
    let first = chunk.x * CHUNK;
    let last = min(first + CHUNK, tile_count);
    var at = LIST + bins[2u + slot] + chunk_counts[chunk.x * MAX_SLOTS + slot];
    for (var tile = first; tile < last; tile = tile + 1u) {
        if (covers(tile, slot)) {
            bins[at] = tile;
            at = at + 1u;
        }
    }
}
