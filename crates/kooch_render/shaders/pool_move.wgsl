// Carries GPU-written meshes to the ranges a pool repack gave them (#773). Vertices and triangle
// bytes hold no pool index, so they go by plain copy; these two arrays do, and are rebased here.

struct Job {
    from_mv: u32,
    to_mv: u32,
    mv_len: u32,
    // Each delta is a wrapping u32: adding it moves an index by a signed amount.
    dv: u32,
    from_m: u32,
    to_m: u32,
    m_len: u32,
    dmv: u32,
    dt: u32,
    dm: u32,
    _pad0: u32,
    _pad1: u32,
}

// `MeshletDescriptor`: word 0 indexes meshlet_vertices, 1 the triangle bytes, 7 a parent meshlet.
const WORDS: u32 = 28u;
const NONE: u32 = 0xffffffffu;
const LANES: u32 = 64u;
const ROW: u32 = 65535u;

@group(0) @binding(0) var<storage, read> jobs: array<Job>;
@group(0) @binding(1) var<storage, read> old_mv: array<u32>;
@group(0) @binding(2) var<storage, read_write> new_mv: array<u32>;
@group(0) @binding(3) var<storage, read> old_m: array<u32>;
@group(0) @binding(4) var<storage, read_write> new_m: array<u32>;

@compute @workgroup_size(64)
fn move_main(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let at = wid.y * ROW + wid.x;
    if (at >= arrayLength(&jobs)) {
        return;
    }
    let job = jobs[at];
    for (var i = lid; i < job.mv_len; i += LANES) {
        new_mv[job.to_mv + i] = old_mv[job.from_mv + i] + job.dv;
    }
    for (var i = lid; i < job.m_len * WORDS; i += LANES) {
        var word = old_m[job.from_m * WORDS + i];
        switch (i % WORDS) {
            case 0u: { word += job.dmv; }
            case 1u: { word += job.dt; }
            case 7u: { if (word != NONE) { word += job.dm; } }
            default: {}
        }
        new_m[job.to_m * WORDS + i] = word;
    }
}
