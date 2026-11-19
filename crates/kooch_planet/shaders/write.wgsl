// Planet meshing, second pass (#313): each block's vertices and grid meshlets, written straight
// into the ranges the meshlet pool reserved for it. Appended to nets.wgsl.

struct Job {
    slot: u32,
    voxel: f32,
    first_vertex: u32,
    first_meshlet_vertex: u32,
    first_byte: u32,
    first_meshlet: u32,
    // What the count pass found; a block that meshes otherwise is not written.
    vertices: u32,
    meshlets: u32,
    meshlet_vertices: u32,
    bytes: u32,
    _pad0: u32,
    _pad1: u32,
}

const MAP_WORDS: u32 = 2457u;
// `MeshVertex` is eight floats: position, normal, uv. `MeshletDescriptor` is 28 words.
const VERTEX_FLOATS: u32 = 8u;
const MESHLET_WORDS: u32 = 28u;
const NONE: u32 = 0xffffffffu;

@group(0) @binding(1) var<storage, read> jobs: array<Job>;
@group(0) @binding(2) var<storage, read_write> pool_vertices: array<f32>;
@group(0) @binding(3) var<storage, read_write> pool_meshlet_vertices: array<u32>;
@group(0) @binding(4) var<storage, read_write> pool_triangles: array<u32>;
@group(0) @binding(5) var<storage, read_write> pool_meshlets: array<u32>;

// Cell → its vertex + 1, two 16-bit entries per word; 0 is no vertex.
var<workgroup> map: array<atomic<u32>, MAP_WORDS>;
var<workgroup> counter: atomic<u32>;
var<workgroup> plans: array<Plan, REGIONS>;
// Per region: its first meshlet, meshlet vertex and triangle byte, within the block.
var<workgroup> starts: array<vec3<u32>, REGIONS>;
var<workgroup> fits: u32;
var<workgroup> job: Job;

fn vertex_of(i: u32) -> u32 {
    let word = atomicLoad(&map[i >> 1u]);
    return (word >> ((i & 1u) * 16u)) & 0xffffu;
}

fn position(v: u32) -> vec3<f32> {
    let o = v * VERTEX_FLOATS;
    return vec3<f32>(pool_vertices[o], pool_vertices[o + 1u], pool_vertices[o + 2u]);
}

// One meshlet from the region at `base`: its vertex set, its triangles, its descriptor.
fn emit(base: vec3<u32>, side: u32, at: vec3<u32>) {
    var mask: array<u32, 4>;
    let quads = fill(base, side, &mask);
    let first_mv = job.first_meshlet_vertex + at.y;
    let first_byte = job.first_byte + at.z;
    var lo = vec3<f32>(3.4e38);
    var hi = vec3<f32>(-3.4e38);
    var r = 0u;
    for (var bit = 0u; bit < 125u; bit++) {
        if (((mask[bit >> 5u] >> (bit & 31u)) & 1u) == 0u) {
            continue;
        }
        let v = job.first_vertex + vertex_of(cell_index(bit_cell(bit, base, side))) - 1u;
        pool_meshlet_vertices[first_mv + r] = v;
        let p = position(v);
        lo = min(lo, p);
        hi = max(hi, p);
        r += 1u;
    }
    var word = 0u;
    var packed = 0u;
    var written = 0u;
    for (var i = 0u; i < side * side * side; i++) {
        let s = base + vec3<u32>(i % side, (i / side) % side, i / (side * side));
        for (var axis = 0u; axis < 3u; axis++) {
            let found = edge(s, axis);
            if (found.x == 0u) {
                continue;
            }
            let cells = quad(s, axis);
            var q: array<u32, 4>;
            for (var k = 0u; k < 4u; k++) {
                q[k] = rank(&mask, cell_bit(cells[k], base, side));
            }
            // Starting inside, the surface faces +axis; outside, the other way.
            var corners6 = array<u32, 6>(q[0], q[1], q[2], q[0], q[2], q[3]);
            if (found.y == 0u) {
                corners6 = array<u32, 6>(q[0], q[2], q[1], q[0], q[3], q[2]);
            }
            for (var k = 0u; k < 6u; k++) {
                packed |= corners6[k] << (8u * (written & 3u));
                written += 1u;
                if ((written & 3u) == 0u) {
                    pool_triangles[(first_byte >> 2u) + word] = packed;
                    word += 1u;
                    packed = 0u;
                }
            }
        }
    }
    if ((written & 3u) != 0u) {
        pool_triangles[(first_byte >> 2u) + word] = packed;
    }
    let center = (lo + hi) * 0.5;
    let o = (job.first_meshlet + at.x) * MESHLET_WORDS;
    pool_meshlets[o] = first_mv;
    pool_meshlets[o + 1u] = first_byte;
    pool_meshlets[o + 2u] = r;
    pool_meshlets[o + 3u] = quads * 2u;
    pool_meshlets[o + 4u] = bitcast<u32>(lo.x);
    pool_meshlets[o + 5u] = bitcast<u32>(lo.y);
    pool_meshlets[o + 6u] = bitcast<u32>(lo.z);
    pool_meshlets[o + 7u] = NONE;
    pool_meshlets[o + 8u] = bitcast<u32>(hi.x);
    pool_meshlets[o + 9u] = bitcast<u32>(hi.y);
    pool_meshlets[o + 10u] = bitcast<u32>(hi.z);
    pool_meshlets[o + 11u] = 0u;
    pool_meshlets[o + 12u] = bitcast<u32>(center.x);
    pool_meshlets[o + 13u] = bitcast<u32>(center.y);
    pool_meshlets[o + 14u] = bitcast<u32>(center.z);
    pool_meshlets[o + 15u] = bitcast<u32>(length(hi - lo) * 0.5);
    // A cutoff of one turns the normal cone off: terrain seldom faces wholly away.
    pool_meshlets[o + 16u] = bitcast<u32>(center.x);
    pool_meshlets[o + 17u] = bitcast<u32>(center.y);
    pool_meshlets[o + 18u] = bitcast<u32>(center.z);
    pool_meshlets[o + 19u] = bitcast<u32>(1.0);
    pool_meshlets[o + 20u] = 0u;
    pool_meshlets[o + 21u] = 0u;
    pool_meshlets[o + 22u] = 0u;
    pool_meshlets[o + 23u] = NONE;
    pool_meshlets[o + 24u] = NONE;
    pool_meshlets[o + 25u] = 0u;
    pool_meshlets[o + 26u] = 0u;
    pool_meshlets[o + 27u] = 0u;
}

@compute @workgroup_size(256)
fn write_main(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    if (lid == 0u) {
        job = jobs[wid.x];
        corner = tile_corner(job.slot);
        atomicStore(&counter, 0u);
        fits = 0u;
    }
    for (var i = lid; i < MAP_WORDS; i += WORKGROUP) {
        atomicStore(&map[i], 0u);
    }
    workgroupBarrier();

    for (var i = lid; i < CELL_COUNT; i += WORKGROUP) {
        if (crosses(corners(cell_of(i)))) {
            let n = atomicAdd(&counter, 1u) + 1u;
            atomicOr(&map[i >> 1u], n << ((i & 1u) * 16u));
        }
    }
    if (lid < REGIONS) {
        plans[lid] = plan(lid);
    }
    workgroupBarrier();
    if (lid == 0u) {
        var at = vec3<u32>(0u);
        for (var q = 0u; q < REGIONS; q++) {
            starts[q] = at;
            at += vec3<u32>(plans[q].meshlets, plans[q].vertices, plans[q].bytes);
        }
        let reserved = vec4<u32>(job.vertices, job.meshlets, job.meshlet_vertices, job.bytes);
        let found = vec4<u32>(atomicLoad(&counter), at);
        fits = select(0u, 1u, all(found == reserved));
    }
    // Writing past what was counted would land in another mesh's ranges.
    if (workgroupUniformLoad(&fits) == 0u) {
        return;
    }

    for (var i = lid; i < CELL_COUNT; i += WORKGROUP) {
        let n = vertex_of(i);
        if (n == 0u) {
            continue;
        }
        let c = cell_of(i);
        let placed = place(corners(c));
        // Sample 1 is the block's corner.
        let p = (vec3<f32>(c) + placed[0] - vec3<f32>(1.0)) * job.voxel;
        let o = (job.first_vertex + n - 1u) * VERTEX_FLOATS;
        pool_vertices[o] = p.x;
        pool_vertices[o + 1u] = p.y;
        pool_vertices[o + 2u] = p.z;
        pool_vertices[o + 3u] = placed[1].x;
        pool_vertices[o + 4u] = placed[1].y;
        pool_vertices[o + 5u] = placed[1].z;
        pool_vertices[o + 6u] = 0.0;
        pool_vertices[o + 7u] = 0.0;
    }
    storageBarrier();
    workgroupBarrier();

    if (lid < REGIONS && plans[lid].meshlets != 0u) {
        let base = region_base(lid);
        let start = starts[lid];
        if (plans[lid].split == 0u) {
            emit(base, 4u, start);
        } else {
            var at = start;
            var mask: array<u32, 4>;
            for (var e = 0u; e < 8u; e++) {
                let part = eighth_base(base, e);
                let quads = fill(part, 2u, &mask);
                if (quads == 0u) {
                    continue;
                }
                emit(part, 2u, at);
                at += vec3<u32>(1u, ones(&mask), quad_bytes(quads));
            }
        }
    }
}
