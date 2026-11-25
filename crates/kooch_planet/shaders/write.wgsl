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
    // Per region, the bricks its meshlets start at, as the count pass packed them.
    cuts: array<u32, CUT_WORDS>,
    part: u32,
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
// #1999 debug: writes skipped, per part, then per mismatch (vertices, meshlets, meshlet vertices,
// bytes) and packs past MAX_PACKS. Never reset; the CPU logs the difference.
@group(0) @binding(6) var<storage, read_write> skips: array<atomic<u32>, 8>;

// Cell → its vertex + 1, two 16-bit entries per word; 0 is no vertex.
var<workgroup> map: array<atomic<u32>, MAP_WORDS>;
var<workgroup> counter: atomic<u32>;
var<workgroup> fits: u32;
var<workgroup> job: Job;

// Every meshlet of the block, one a thread (#1938): a region's meshlets hold two bricks or more
// but the last, so 8 × 33, and a patch each.
const MAX_PACKS: u32 = 312u;
// Region 6 bits, first brick 6, end 7, quads 6, cells 7.
var<workgroup> packs: array<u32, MAX_PACKS>;
// Its first meshlet vertex (15 bits) and triangle byte (17), within the block.
var<workgroup> pack_at: array<u32, MAX_PACKS>;
// Per patch, its cells and bytes << 8; zero when it has no meshlet.
var<workgroup> patch_sizes: array<u32, 48>;
var<workgroup> listed: u32;
// Per mask slot, its set bits before each word: a corner's rank in one popcount.
var<workgroup> before: array<Mask, MASK_SLOTS>;

// Builds meshlet `t`'s mask and ranks in `slot`; its quads and cells.
fn build_pack(t: u32, slot: u32) -> vec2<u32> {
    let info = packs[t];
    let origin = region(info & 63u).base;
    for (var w = 0u; w < PACK_WORDS; w++) {
        masks[slot][w] = 0u;
    }
    var quads = 0u;
    for (var m = (info >> 6u) & 63u; m < ((info >> 12u) & 127u); m++) {
        quads += add_brick(origin, m, slot);
    }
    var cells = 0u;
    for (var w = 0u; w < PACK_WORDS; w++) {
        before[slot][w] = cells;
        cells += countOneBits(masks[slot][w]);
    }
    return vec2<u32>(quads, cells);
}

fn pack_info(q: u32, first: u32, next: u32, quads: u32, cells: u32) -> u32 {
    return q | (first << 6u) | (next << 12u) | (quads << 19u) | (cells << 25u);
}

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

// Meshlet `at.x`: bricks `first..next` of region `q`, its cells in mask `slot` and their ranks in
// `before[slot]`.
fn emit_pack(q: u32, first: u32, next: u32, quads: u32, at: vec3<u32>, slot: u32) {
    let origin = region(q).base;
    let first_mv = job.first_meshlet_vertex + at.y;
    var lo = vec3<f32>(3.4e38);
    var hi = vec3<f32>(-3.4e38);
    var r = 0u;
    for (var w = 0u; w < PACK_WORDS; w++) {
        var word = masks[slot][w];
        while (word != 0u) {
            let bit = w * 32u + countTrailingZeros(word);
            word &= word - 1u;
            let c = origin - vec3<u32>(1u) + vec3<u32>(bit % 9u, (bit / 9u) % 9u, bit / 81u);
            put_vertex(first_mv, r, c, &lo, &hi);
            r += 1u;
        }
    }
    var t = Strip(job.first_byte + at.z, 0u, 0u, 0u);
    for (var m = first; m < next; m++) {
        let b = brick_base(origin, m);
        var word = brick_edges(b);
        let inside = brick_inside(b);
        while (word != 0u) {
            let bit = countTrailingZeros(word);
            let e = brick_edge(b, bit);
            word &= word - 1u;
            let cells = quad(e.xyz, e.w);
            var corners: array<u32, 4>;
            for (var k = 0u; k < 4u; k++) {
                let cell = pack_bit(cells[k], origin);
                let low = masks[slot][cell >> 5u] & ((1u << (cell & 31u)) - 1u);
                corners[k] = before[slot][cell >> 5u] + countOneBits(low);
            }
            put_quad(&t, corners, ((inside >> (bit & 7u)) & 1u) != 0u);
        }
    }
    end_strip(&t);
    describe(at.x, first_mv, t.first, r, quads, lo, hi);
}

// The block's inner meshlets: those its cuts start.
fn inner_count() -> u32 {
    var n = 0u;
    for (var w = 0u; w < CUT_WORDS; w++) {
        n += countOneBits(job.cuts[w]);
    }
    return n;
}

// Lists region `q`'s meshlets from its cuts, each running to the next cut or the region's end.
fn list_region(q: u32) {
    var n = 0u;
    for (var w = 0u; w < 2u * q; w++) {
        n += countOneBits(job.cuts[w]);
    }
    let cuts = vec2<u32>(job.cuts[2u * q], job.cuts[2u * q + 1u]);
    var first = 64u;
    for (var m = 0u; m < 64u; m++) {
        if (((cuts[m >> 5u] >> (m & 31u)) & 1u) == 0u) {
            continue;
        }
        if (first < 64u) {
            packs[n] = pack_info(q, first, m, 0u, 0u);
            n += 1u;
        }
        first = m;
    }
    if (first < 64u) {
        packs[n] = pack_info(q, first, 64u, 0u, 0u);
    }
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
        part = job.part;
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
    mark_bricks(lid);
    workgroupBarrier();

    for (var i = lid; i < CELL_COUNT; i += WORKGROUP) {
        if (has_vertex(cell_of(i))) {
            let n = atomicAdd(&counter, 1u) + 1u;
            atomicOr(&map[i >> 1u], n << ((i & 1u) * 16u));
        }
    }
    let inner = inner_count();
    if (lid < INNER) {
        list_region(lid);
    } else if (lid < REGIONS) {
        var unused = vec2<u32>(0u);
        let p = plan(lid, &unused);
        patch_sizes[lid - INNER] = select(0u, p.vertices | (p.bytes << 8u), p.meshlets != 0u);
    }
    workgroupBarrier();
    // Each meshlet's quads and cells, one a slot. With no more meshlets than slots, the masks stay
    // for the emit below; past that, each is built again there.
    for (var t = lid; lid < MASK_SLOTS && t < min(inner, MAX_PACKS); t += MASK_SLOTS) {
        let size = build_pack(t, lid);
        packs[t] |= (size.x << 19u) | (size.y << 25u);
    }
    workgroupBarrier();
    if (lid == 0u) {
        var n = min(inner, MAX_PACKS);
        var mv = 0u;
        var bytes = 0u;
        for (var t = 0u; t < n; t++) {
            pack_at[t] = mv | (bytes << 15u);
            mv += packs[t] >> 25u;
            bytes += quad_bytes((packs[t] >> 19u) & 63u);
        }
        for (var k = 0u; k < 48u; k++) {
            let size = patch_sizes[k];
            if (size == 0u || n >= MAX_PACKS) {
                continue;
            }
            packs[n] = pack_info(INNER + k, 0u, 0u, 0u, 0u);
            pack_at[n] = mv | (bytes << 15u);
            mv += size & 0xffu;
            bytes += size >> 8u;
            n += 1u;
        }
        listed = n;
        let reserved = vec4<u32>(job.vertices, job.meshlets, job.meshlet_vertices, job.bytes);
        let found = vec4<u32>(atomicLoad(&counter), n, mv, bytes);
        fits = select(0u, 1u, all(found == reserved) && inner <= MAX_PACKS);
        if (fits == 0u) {
            atomicAdd(&skips[min(part, 2u)], 1u);
            for (var k = 0u; k < 4u; k++) {
                atomicAdd(&skips[3u + k], select(0u, 1u, found[k] != reserved[k]));
            }
            atomicAdd(&skips[7u], select(0u, 1u, inner > MAX_PACKS));
        }
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
        let sunk = placed[0] - placed[1] * select(0.0, SINK, part == OVERLAP);
        // Sample 1 is the block's corner.
        let p = (sunk - vec3<f32>(1.0)) * job.voxel;
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

    for (var t = lid; lid < MASK_SLOTS && t < listed; t += MASK_SLOTS) {
        let info = packs[t];
        let q = info & 63u;
        let at = vec3<u32>(t, pack_at[t] & 0x7fffu, pack_at[t] >> 15u);
        if (q >= INNER) {
            emit(region(q), at);
            continue;
        }
        if (inner > MASK_SLOTS) {
            build_pack(t, lid);
        }
        emit_pack(q, (info >> 6u) & 63u, (info >> 12u) & 127u, (info >> 19u) & 63u, at, lid);
    }
}
