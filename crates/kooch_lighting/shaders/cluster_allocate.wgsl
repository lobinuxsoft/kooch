// Pass 3 of 4: where each cell's list of indices starts (#780).
//
// The index list is one buffer of tightly packed, variable-length runs —
// one run per cell. Knowing where a cell's run begins means summing the
// lengths of every cell before it, which is a prefix sum.
//
// Workgroups cap at 256 invocations and a grid has thousands of cells,
// so it takes two dispatches: a [Hillis-Steele scan] within each block
// of 256, then a sequential march across the blocks to carry the running
// total. Same two-step Bevy uses, and for the same reason.
//
// [Hillis-Steele scan]: https://en.wikipedia.org/wiki/Prefix_sum
//
// Concatenated after `cluster_common.wgsl`.

@group(0) @binding(0) var<uniform> cluster_view: ClusterView;
@group(0) @binding(2) var<storage, read_write> cluster_draw: ClusterDraw;
@group(0) @binding(4) var<storage, read_write> cluster_cells: array<ClusterCell>;
@group(0) @binding(5) var<storage, read_write> cluster_scratch: array<ClusterCell>;

const BLOCK: u32 = 256u;

// Each thread's offset relative to the start of its block.
var<workgroup> block_offsets: array<u32, 256>;

// Within one block of 256 cells, the offset of each cell relative to the
// block's first.
@compute @workgroup_size(256, 1, 1)
fn allocate_local_main(
    @builtin(local_invocation_id) local_id: vec3<u32>,
    @builtin(workgroup_id) group_id: vec3<u32>,
    @builtin(global_invocation_id) global_id: vec3<u32>,
) {
    let cell_count = cluster_view.dimensions.w;
    let block_end = min(group_id.x * BLOCK + BLOCK, cell_count);
    let local = local_id.x;

    block_offsets[local] = 0u;
    workgroupBarrier();

    // Shifted by one: a cell's offset is the sum of the cells BEFORE it,
    // so thread `n` is seeded with cell `n - 1`'s length and the scan
    // turns that into an exclusive prefix sum.
    if (global_id.x < block_end && local < BLOCK - 1u) {
        block_offsets[local + 1u] = cell_total(global_id.x);
    }
    workgroupBarrier();

    for (var stride = 1u; stride < BLOCK; stride = stride * 2u) {
        var term = 0u;
        if (local >= stride) {
            term = block_offsets[local - stride];
        }
        // 🔴 Both barriers are load-bearing. The first stops a thread
        // from overwriting a slot another one has not read yet; the
        // second stops a thread from reading a slot before its write
        // lands. Dropping either produces a scan that is right on one
        // driver and wrong on the next.
        workgroupBarrier();
        block_offsets[local] = block_offsets[local] + term;
        workgroupBarrier();
    }

    if (global_id.x < block_end) {
        // What this cell ended up holding, while a thread is already
        // here and the count is final (#820). The populate pass has not
        // run yet, so this is the counting pass's verdict — which is
        // exactly the number the shading loop will walk.
        let total = cell_total(global_id.x);
        atomicMax(&cluster_draw.peak_cell, total);
        if (total > 0u) {
            atomicAdd(&cluster_draw.filled_cells, 1u);
        }
        atomicStore(&cluster_cells[global_id.x].offset, block_offsets[local]);
        // Zero the scratch counters while a thread is already here: the
        // populate pass counts back up from them, and a stale count
        // would write this frame's indices past the end of the run.
        clear_scratch(global_id.x);
    }
}

// Carries each block's running total into the blocks that follow.
//
// One workgroup, marching the blocks in order. Sequential by nature —
// block `n`'s base is block `n-1`'s base plus its length — which is why
// it is a second dispatch rather than more threads.
@compute @workgroup_size(256, 1, 1)
fn allocate_global_main(@builtin(local_invocation_id) local_id: vec3<u32>) {
    let cell_count = cluster_view.dimensions.w;

    var carry = 0u;
    for (var base = 0u; base < cell_count; base = base + BLOCK) {
        let cell = base + local_id.x;
        if (cell < cell_count) {
            let offset = atomicLoad(&cluster_cells[cell].offset) + carry;
            atomicStore(&cluster_cells[cell].offset, offset);
        }
        storageBarrier();

        if (base + BLOCK - 1u < cell_count) {
            let last = base + BLOCK - 1u;
            carry = atomicLoad(&cluster_cells[last].offset) + cell_total(last);
        }
        storageBarrier();
    }

    // What the whole grid needs, for the CPU to size the buffer with.
    if (local_id.x == 0u) {
        let last = cell_count - 1u;
        cluster_draw.index_size = atomicLoad(&cluster_cells[last].offset) + cell_total(last);
    }
}

// How many indices a cell holds, across all five types.
fn cell_total(cell: u32) -> u32 {
    return atomicLoad(&cluster_cells[cell].point_count)
        + atomicLoad(&cluster_cells[cell].spot_count)
        + atomicLoad(&cluster_cells[cell].probe_count)
        + atomicLoad(&cluster_cells[cell].volume_count)
        + atomicLoad(&cluster_cells[cell].decal_count);
}

fn clear_scratch(cell: u32) {
    atomicStore(&cluster_scratch[cell].point_count, 0u);
    atomicStore(&cluster_scratch[cell].spot_count, 0u);
    atomicStore(&cluster_scratch[cell].probe_count, 0u);
    atomicStore(&cluster_scratch[cell].volume_count, 0u);
    atomicStore(&cluster_scratch[cell].decal_count, 0u);
}
