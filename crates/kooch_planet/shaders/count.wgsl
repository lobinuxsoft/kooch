// Planet meshing, first pass (#313): what each block's mesh will take of the pool, so its ranges
// can be reserved before the write pass fills them. Appended to nets.wgsl.

struct CountJob {
    slot: u32,
    near: Near,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(1) var<storage, read> jobs: array<CountJob>;
// Per block: vertices, meshlets, meshlet vertices, triangle bytes.
@group(0) @binding(2) var<storage, read_write> counts: array<u32>;

var<workgroup> totals: array<atomic<u32>, 4>;

@compute @workgroup_size(256)
fn count_main(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let block = wid.x;
    if (lid == 0u) {
        corner = tile_corner(jobs[block].slot);
        load_near(jobs[block].near);
    }
    if (lid < 4u) {
        atomicStore(&totals[lid], 0u);
    }
    clear_edges(lid);
    workgroupBarrier();
    mark_edges(lid);
    workgroupBarrier();
    for (var i = lid; i < CELL_COUNT; i += WORKGROUP) {
        if (has_vertex(cell_of(i))) {
            atomicAdd(&totals[0], 1u);
        }
    }
    if (lid < REGIONS) {
        let p = plan(lid);
        atomicAdd(&totals[1], p.meshlets);
        atomicAdd(&totals[2], p.vertices);
        atomicAdd(&totals[3], p.bytes);
    }
    workgroupBarrier();
    if (lid < 4u) {
        counts[block * 4u + lid] = atomicLoad(&totals[lid]);
    }
}
