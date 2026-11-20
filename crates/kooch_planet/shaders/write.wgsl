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
    near: Near,
    _pad: u32,
}

const MAP_WORDS: u32 = 2916u;
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

// A meshlet's triangles as bytes, packed four to a word from `first_byte`.
struct Strip {
    first: u32,
    word: u32,
    packed: u32,
    written: u32,
}

fn put_quad(t: ptr<function, Strip>, q: array<u32, 4>, inside: bool) {
    // Starting inside, the surface faces +axis; outside, the other way.
    var corners6 = array<u32, 6>(q[0], q[1], q[2], q[0], q[2], q[3]);
    if (!inside) {
        corners6 = array<u32, 6>(q[0], q[2], q[1], q[0], q[3], q[2]);
    }
    for (var k = 0u; k < 6u; k++) {
        (*t).packed |= corners6[k] << (8u * ((*t).written & 3u));
        (*t).written += 1u;
        if (((*t).written & 3u) == 0u) {
            pool_triangles[((*t).first >> 2u) + (*t).word] = (*t).packed;
            (*t).word += 1u;
            (*t).packed = 0u;
        }
    }
}

fn end_strip(t: ptr<function, Strip>) {
    if (((*t).written & 3u) != 0u) {
        pool_triangles[((*t).first >> 2u) + (*t).word] = (*t).packed;
    }
}

// The meshlet's vertex `r`: the pool vertex of cell `c`, grown into the bounds.
fn put_vertex(first_mv: u32, r: u32, c: vec3<u32>, lo: ptr<function, vec3<f32>>, hi: ptr<function, vec3<f32>>) {
    let v = job.first_vertex + vertex_of(cell_index(c)) - 1u;
    pool_meshlet_vertices[first_mv + r] = v;
    let p = position(v);
    *lo = min(*lo, p);
    *hi = max(*hi, p);
}

// One patch as a meshlet: its vertex set, its triangles, its descriptor.
fn emit(cut: Region, at: vec3<u32>) {
    var mask: array<u32, 4>;
    let quads = fill(cut, &mask);
    let first_mv = job.first_meshlet_vertex + at.y;
    var lo = vec3<f32>(3.4e38);
    var hi = vec3<f32>(-3.4e38);
    var r = 0u;
    for (var bit = 0u; bit < bits(cut); bit++) {
        if (((mask[bit >> 5u] >> (bit & 31u)) & 1u) != 0u) {
            put_vertex(first_mv, r, bit_cell(bit, cut), &lo, &hi);
            r += 1u;
        }
    }
    var t = Strip(job.first_byte + at.z, 0u, 0u, 0u);
    for (var i = 0u; i < cut.size.x * cut.size.y * cut.size.z; i++) {
        let s = start_of(i, cut);
        for (var axis = 0u; axis < 3u; axis++) {
            let found = emits(s, axis);
            if (found.x == 0u) {
                continue;
            }
            let cells = quad(s, axis);
            var q: array<u32, 4>;
            for (var k = 0u; k < 4u; k++) {
                q[k] = rank(&mask, cell_bit(cells[k], cut));
            }
            put_quad(&t, q, found.y != 0u);
        }
    }
    end_strip(&t);
    describe(at.x, first_mv, t.first, r, quads, lo, hi);
}

// One packed meshlet of the region at `origin`.
fn emit_pack(origin: vec3<u32>, p: ptr<function, Pack>, at: vec3<u32>) {
    let first_mv = job.first_meshlet_vertex + at.y;
    var lo = vec3<f32>(3.4e38);
    var hi = vec3<f32>(-3.4e38);
    var r = 0u;
    for (var w = 0u; w < PACK_WORDS; w++) {
        var word = (*p).mask[w];
        while (word != 0u) {
            let bit = w * 32u + countTrailingZeros(word);
            word &= word - 1u;
            let c = origin - vec3<u32>(1u) + vec3<u32>(bit % 9u, (bit / 9u) % 9u, bit / 81u);
            put_vertex(first_mv, r, c, &lo, &hi);
            r += 1u;
        }
    }
    var t = Strip(job.first_byte + at.z, 0u, 0u, 0u);
    for (var n = 0u; n < (*p).count; n++) {
        let b = brick_base(origin, (*p).bricks[n]);
        var word = brick_edges(b);
        while (word != 0u) {
            let e = brick_edge(b, countTrailingZeros(word));
            word &= word - 1u;
            let cells = quad(e.xyz, e.w);
            var q: array<u32, 4>;
            for (var k = 0u; k < 4u; k++) {
                q[k] = pack_rank(p, pack_bit(cells[k], origin));
            }
            put_quad(&t, q, edge(e.xyz, e.w).y != 0u);
        }
    }
    end_strip(&t);
    describe(at.x, first_mv, t.first, r, (*p).quads, lo, hi);
}

fn describe(index: u32, first_mv: u32, first_byte: u32, r: u32, quads: u32, lo: vec3<f32>, hi: vec3<f32>) {
    let center = (lo + hi) * 0.5;
    let o = (job.first_meshlet + index) * MESHLET_WORDS;
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
        load_near(job.near);
        atomicStore(&counter, 0u);
        fits = 0u;
    }
    for (var i = lid; i < MAP_WORDS; i += WORKGROUP) {
        atomicStore(&map[i], 0u);
    }
    clear_edges(lid);
    workgroupBarrier();
    mark_edges(lid);
    workgroupBarrier();

    for (var i = lid; i < CELL_COUNT; i += WORKGROUP) {
        if (has_vertex(cell_of(i))) {
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
        let placed = cell_vertex(cell_of(i));
        // Sample 1 is the block's corner.
        let p = (placed[0] - vec3<f32>(1.0)) * job.voxel;
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
        let start = starts[lid];
        if (lid >= INNER) {
            emit(region(lid), start);
        } else {
            let origin = region(lid).base;
            var p: Pack;
            var at = start;
            while (next_meshlet(origin, &p)) {
                emit_pack(origin, &p, at);
                at += vec3<u32>(1u, p.cells, quad_bytes(p.quads));
            }
        }
    }
}
