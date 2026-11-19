// Surface nets over one block's tile (#313), shared by the count and write passes. Both walk the
// same regions the same way, so the sizes one counts are the sizes the other fills.
//
// A cell holds a vertex when its eight samples change sign: the mean of its edge crossings. Each
// sign-changing edge whose start lies in the block gives a quad over the four cells around it, so
// two blocks of one level share their border cells and the mesh closes between them.
//
// Across a coarser leaf (#313 step 4c) a border cell is the coarse cell holding it, read from the
// coarse tile, and a sample on the shared border is the coarsest leaf's: every leaf there agrees on
// its sign. The finer leaf emits the seam: a block also emits its +plane edges where the leaf past
// them is coarser, and none whose cells reach a finer one.
//
// Meshlets are packed, not cut at fixed planes (#1935): the block's 16³ edge starts are 2³ bricks,
// and each 8³ region packs its non-empty bricks in Morton order into a meshlet until the next one
// would pass 62 quads or 64 cells. A brick alone always fits. The +plane edges take 16 patches a
// face, one sample deep, one meshlet each.

override TILES_X: u32 = 32u;
override TILES_Y: u32 = 1u;

// The tile is 18 samples: the block's 16, and one either side. Cells sit between samples; cell 17
// is past the tile, and only a coarser leaf there gives it a vertex.
const TILE: u32 = 18u;
const CELLS: u32 = 18u;
const CELL_COUNT: u32 = 5832u;
const WORKGROUP: u32 = 256u;
const INNER: u32 = 8u;
const REGIONS: u32 = 56u;
const MAX_QUADS: u32 = 62u;
// An 8³ region's cells, 9³ bits.
const PACK_WORDS: u32 = 23u;
const MAX_TRIANGLES: u32 = 124u;
const MAX_VERTICES: u32 = 64u;

@group(0) @binding(0) var atlas: texture_3d<f32>;

// Who holds the cells past each of the block's 26 neighbour directions; see `seam.rs`.
struct Near {
    cell: array<i32, 3>,
    sides: array<u32, 26>,
}

const SLOT_MASK: u32 = 0xffffffu;
const FINER: u32 = 0x80000000u;

var<workgroup> near: Near;
// Whether any side is a coarser leaf, or a finer one: most blocks have neither.
var<workgroup> borrows: u32;
var<workgroup> finer: u32;

// Run by one invocation before the barrier that precedes any sample read.
fn load_near(n: Near) {
    near = n;
    borrows = 0u;
    finer = 0u;
    for (var d = 0u; d < 26u; d++) {
        borrows |= select(0u, 1u, levels(n.sides[d]) != 0u);
        finer |= select(0u, 1u, n.sides[d] == FINER);
    }
}

fn levels(side: u32) -> u32 {
    return (side >> 24u) & 0x7fu;
}

fn side_of(d: vec3<i32>) -> u32 {
    let i = (d.x + 1) + 3 * (d.y + 1) + 9 * (d.z + 1);
    return near.sides[u32(i) - select(0u, 1u, i > 13)];
}

// Where the coarser leaf `up` levels above, across `d`, has its sample 1, in samples of ours from
// our sample 1.
fn coarse_base(d: vec3<i32>, up: u32) -> vec3<i32> {
    let k = vec3<i32>(near.cell[0], near.cell[1], near.cell[2]);
    let kc = (k + d) >> vec3<u32>(up);
    return ((kc * 16) << vec3<u32>(up)) - k * 16;
}

// A coarse tile's sample, in our voxels: tiles store distance over their own voxel.
fn coarse_at(side: u32, t: vec3<i32>) -> f32 {
    let scale = f32(1u << levels(side));
    return textureLoad(atlas, tile_corner(side & SLOT_MASK) + t, 0).x * scale;
}

var<workgroup> corner: vec3<i32>;

fn tile_corner(slot: u32) -> vec3<i32> {
    let tile = vec3<u32>(slot % TILES_X, (slot / TILES_X) % TILES_Y, slot / (TILES_X * TILES_Y));
    return vec3<i32>(tile * TILE);
}

fn own(s: vec3<u32>) -> f32 {
    return textureLoad(atlas, corner + vec3<i32>(s), 0).x;
}

fn at(s: vec3<u32>) -> f32 {
    let border = any(s == vec3<u32>(1u)) || any(s == vec3<u32>(TILE - 1u));
    if (borrows == 0u || !border) {
        return own(s);
    }
    return shared_at(s);
}

// A border sample from the coarsest leaf around it that has it as a sample.
fn shared_at(s: vec3<u32>) -> f32 {
    var best = 0u;
    var value = own(s);
    for (var m = 1u; m < 8u; m++) {
        var d = vec3<i32>(0);
        var around = true;
        for (var a = 0u; a < 3u; a++) {
            if (((m >> a) & 1u) == 0u) {
                continue;
            }
            if (s[a] == 1u) {
                d[a] = -1;
            } else if (s[a] == TILE - 1u) {
                d[a] = 1;
            } else {
                around = false;
            }
        }
        let side = side_of(d);
        let up = levels(side);
        if (!around || up <= best) {
            continue;
        }
        let rel = vec3<i32>(s) - vec3<i32>(1) - coarse_base(d, up);
        let unaligned = rel & vec3<i32>((1 << up) - 1);
        if (any(unaligned != vec3<i32>(0))) {
            continue;
        }
        best = up;
        value = coarse_at(side, (rel >> vec3<u32>(up)) + vec3<i32>(1));
    }
    return value;
}

fn cell_of(i: u32) -> vec3<u32> {
    return vec3<u32>(i % CELLS, (i / CELLS) % CELLS, i / (CELLS * CELLS));
}

fn cell_index(c: vec3<u32>) -> u32 {
    return c.x + c.y * CELLS + c.z * CELLS * CELLS;
}

fn corners(c: vec3<u32>) -> array<f32, 8> {
    var v: array<f32, 8>;
    for (var k = 0u; k < 8u; k++) {
        v[k] = at(c + vec3<u32>(k & 1u, (k >> 1u) & 1u, k >> 2u));
    }
    return v;
}

fn crosses(v: array<f32, 8>) -> bool {
    var inside = 0u;
    for (var k = 0u; k < 8u; k++) {
        inside += select(0u, 1u, v[k] < 0.0);
    }
    return inside != 0u && inside != 8u;
}

// The mean of the cell's edge crossings, within the cell, and the gradient as its normal. A cell
// with none, which only a finer neighbour borrows, takes one Newton step from its centre.
fn place(v: array<f32, 8>) -> array<vec3<f32>, 2> {
    var sum = vec3<f32>(0.0);
    var count = 0.0;
    for (var k = 0u; k < 8u; k++) {
        for (var axis = 0u; axis < 3u; axis++) {
            let bit = 1u << axis;
            if ((k & bit) != 0u) {
                continue;
            }
            let a = v[k];
            let b = v[k | bit];
            if ((a < 0.0) == (b < 0.0)) {
                continue;
            }
            let start = vec3<f32>(f32(k & 1u), f32((k >> 1u) & 1u), f32(k >> 2u));
            var step = vec3<f32>(0.0);
            step[axis] = 1.0;
            sum += start + step * (a / (a - b));
            count += 1.0;
        }
    }
    let g = vec3<f32>(
        (v[1] + v[3] + v[5] + v[7]) - (v[0] + v[2] + v[4] + v[6]),
        (v[2] + v[3] + v[6] + v[7]) - (v[0] + v[1] + v[4] + v[5]),
        (v[4] + v[5] + v[6] + v[7]) - (v[0] + v[1] + v[2] + v[3]),
    );
    let still = !(dot(g, g) > 0.0);
    let normal = select(normalize(g), vec3<f32>(0.0, 1.0, 0.0), still);
    if (count > 0.0) {
        return array<vec3<f32>, 2>(sum / count, normal);
    }
    var mean = 0.0;
    for (var k = 0u; k < 8u; k++) {
        mean += v[k] * 0.125;
    }
    // g is four times the gradient across the cell.
    let newton = select(g * (4.0 * mean / dot(g, g)), vec3<f32>(0.0), still);
    let inside = clamp(vec3<f32>(0.5) - newton, vec3<f32>(0.0), vec3<f32>(1.0));
    return array<vec3<f32>, 2>(inside, normal);
}

// Cell `c`'s vertex, in our samples, and its normal. A cell across a coarser leaf is the coarse cell
// holding it, placed as that leaf places it.
fn cell_vertex(c: vec3<u32>) -> array<vec3<f32>, 2> {
    let d = outside(c);
    let side = side_of(d);
    let up = levels(side);
    if (borrows == 0u || all(d == vec3<i32>(0)) || up == 0u) {
        let placed = place(corners(c));
        return array<vec3<f32>, 2>(vec3<f32>(c) + placed[0], placed[1]);
    }
    let base = coarse_base(d, up);
    let t = ((vec3<i32>(c) - vec3<i32>(1) - base) >> vec3<u32>(up)) + vec3<i32>(1);
    var v: array<f32, 8>;
    for (var k = 0u; k < 8u; k++) {
        let tk = t + vec3<i32>(vec3<u32>(k & 1u, (k >> 1u) & 1u, k >> 2u));
        // On our border the coarse leaf reads the sample as we do; past it, from its own tile.
        let s = base + ((tk - vec3<i32>(1)) << vec3<u32>(up)) + vec3<i32>(1);
        let ours = all(s >= vec3<i32>(1)) && all(s <= vec3<i32>(i32(TILE) - 1));
        if (ours) {
            v[k] = at(vec3<u32>(s));
        } else {
            v[k] = coarse_at(side, tk);
        }
    }
    let placed = place(v);
    let first = base + ((t - vec3<i32>(1)) << vec3<u32>(up)) + vec3<i32>(1);
    return array<vec3<f32>, 2>(vec3<f32>(first) + placed[0] * f32(1u << up), placed[1]);
}

// Which neighbour holds cell `c`, per axis: cell 0 is the − one's, cell 17 the + one's.
fn outside(c: vec3<u32>) -> vec3<i32> {
    let low = select(vec3<i32>(0), vec3<i32>(-1), c == vec3<u32>(0u));
    return select(low, vec3<i32>(1), c == vec3<u32>(CELLS - 1u));
}

// Whether the block emits the edge from `s` along `axis`: none whose cells reach a finer leaf, which
// emits it itself, and past the +plane only those whose cells there are a coarser leaf's.
fn owns(s: vec3<u32>, axis: u32) -> bool {
    if (s[axis] > 16u) {
        return false;
    }
    if (borrows == 0u && finer == 0u) {
        return true;
    }
    let cells = quad(s, axis);
    for (var k = 0u; k < 4u; k++) {
        let d = outside(cells[k]);
        if (all(d == vec3<i32>(0))) {
            continue;
        }
        let side = side_of(d);
        if (side == FINER || (any(d == vec3<i32>(1)) && levels(side) == 0u)) {
            return false;
        }
    }
    return true;
}

// `edge` over the edges the block emits.
fn emits(s: vec3<u32>, axis: u32) -> vec2<u32> {
    if (!owns(s, axis)) {
        return vec2<u32>(0u);
    }
    return edge(s, axis);
}

// Whether cell `c` takes a vertex. Cell 17's samples are past the tile, so it takes one only when an
// emitted edge reaches it.
fn has_vertex(c: vec3<u32>) -> bool {
    if (all(c < vec3<u32>(CELLS - 1u))) {
        return crosses(corners(c));
    }
    if (borrows == 0u) {
        return false;
    }
    for (var axis = 0u; axis < 3u; axis++) {
        let u = (axis + 1u) % 3u;
        let v = (axis + 2u) % 3u;
        for (var k = 0u; k < 4u; k++) {
            var s = c;
            s[u] += k & 1u;
            s[v] += k >> 1u;
            if (all(s >= vec3<u32>(1u)) && all(s <= vec3<u32>(TILE - 1u)) && emits(s, axis).x != 0u) {
                return true;
            }
        }
    }
    return false;
}

// Whether the edge from `s` along `axis` changes sign, and if so whether it starts inside.
fn edge(s: vec3<u32>, axis: u32) -> vec2<u32> {
    var e = vec3<u32>(0u);
    e[axis] = 1u;
    let a = at(s);
    let b = at(s + e);
    return vec2<u32>(select(0u, 1u, (a < 0.0) != (b < 0.0)), select(0u, 1u, a < 0.0));
}

// The four cells around the edge from `s` along `axis`; u × v = axis, so they turn
// counter-clockwise seen from +axis.
fn quad(s: vec3<u32>, axis: u32) -> array<vec3<u32>, 4> {
    var u = vec3<u32>(0u);
    var v = vec3<u32>(0u);
    u[(axis + 1u) % 3u] = 1u;
    v[(axis + 2u) % 3u] = 1u;
    return array<vec3<u32>, 4>(s - u - v, s - v, s, s - u);
}

// Edge starts `base` to `base + size`.
struct Region {
    base: vec3<u32>,
    size: vec3<u32>,
}

// The first 8 are the 8³ regions. Then 16 patches a +plane, each face also taking one line of the
// planes past it, so every start with a 17 in it falls in exactly one: x takes y = 17, y takes z,
// z takes x.
fn region(q: u32) -> Region {
    if (q < INNER) {
        return Region(vec3<u32>(1u) + 8u * vec3<u32>(q & 1u, (q >> 1u) & 1u, q >> 2u), vec3<u32>(8u));
    }
    let face = (q - INNER) / 16u;
    let i = (q - INNER) % 4u;
    let j = ((q - INNER) / 4u) % 4u;
    let a = (face + 1u) % 3u;
    let b = (face + 2u) % 3u;
    var r = Region(vec3<u32>(TILE - 1u), vec3<u32>(1u));
    r.base[a] = 1u + 4u * i;
    r.size[a] = 4u + select(0u, 1u, i == 3u);
    r.base[b] = 1u + 4u * j;
    r.size[b] = 4u;
    return r;
}

// Per axis, whether the block emits the edge from each of its 16³ starts: found once, read by
// every brick that packs and every meshlet that writes.
var<workgroup> edges: array<atomic<u32>, 384>;

fn clear_edges(lid: u32) {
    for (var w = lid; w < 384u; w += WORKGROUP) {
        atomicStore(&edges[w], 0u);
    }
}

fn mark_edges(lid: u32) {
    for (var i = lid; i < 4096u; i += WORKGROUP) {
        let s = vec3<u32>(1u) + vec3<u32>(i % 16u, (i / 16u) % 16u, i / 256u);
        for (var axis = 0u; axis < 3u; axis++) {
            if (emits(s, axis).x != 0u) {
                let n = axis * 4096u + i;
                atomicOr(&edges[n >> 5u], 1u << (n & 31u));
            }
        }
    }
}

fn has_edge(s: vec3<u32>, axis: u32) -> bool {
    let l = s - vec3<u32>(1u);
    let n = axis * 4096u + l.x + l.y * 16u + l.z * 256u;
    return ((atomicLoad(&edges[n >> 5u]) >> (n & 31u)) & 1u) != 0u;
}

// Brick `m` of the region at `origin`, in Morton order so consecutive bricks touch.
fn brick_base(origin: vec3<u32>, m: u32) -> vec3<u32> {
    let x = (m & 1u) | ((m >> 2u) & 2u);
    let y = ((m >> 1u) & 1u) | ((m >> 3u) & 2u);
    let z = ((m >> 2u) & 1u) | ((m >> 4u) & 2u);
    return origin + 2u * vec3<u32>(x, y, z);
}

// A brick's quads, and its 3³ cells they touch as bits.
fn brick(b: vec3<u32>) -> vec2<u32> {
    var quads = 0u;
    var cells = 0u;
    for (var i = 0u; i < 8u; i++) {
        let s = b + vec3<u32>(i & 1u, (i >> 1u) & 1u, i >> 2u);
        for (var axis = 0u; axis < 3u; axis++) {
            if (!has_edge(s, axis)) {
                continue;
            }
            quads += 1u;
            let around = quad(s, axis);
            for (var k = 0u; k < 4u; k++) {
                let l = around[k] + vec3<u32>(1u) - b;
                cells |= 1u << (l.x + 3u * l.y + 9u * l.z);
            }
        }
    }
    return vec2<u32>(quads, cells);
}

// A cell's bit among the 9³ cells of the region at `origin`.
fn pack_bit(c: vec3<u32>, origin: vec3<u32>) -> u32 {
    let l = c + vec3<u32>(1u) - origin;
    return l.x + 9u * l.y + 81u * l.z;
}

// One meshlet of a region: its bricks and cells, and the brick to start the next from.
struct Pack {
    next: u32,
    quads: u32,
    cells: u32,
    count: u32,
    bricks: array<u32, 64>,
    mask: array<u32, PACK_WORDS>,
}

// Packs the next meshlet of the region at `origin` into `p`; false once its bricks are spent.
fn next_meshlet(origin: vec3<u32>, p: ptr<function, Pack>) -> bool {
    (*p).quads = 0u;
    (*p).cells = 0u;
    (*p).count = 0u;
    for (var w = 0u; w < PACK_WORDS; w++) {
        (*p).mask[w] = 0u;
    }
    while ((*p).next < 64u) {
        let b = brick_base(origin, (*p).next);
        let found = brick(b);
        if (found.x == 0u) {
            (*p).next += 1u;
            continue;
        }
        var fresh = 0u;
        for (var k = 0u; k < 27u; k++) {
            if (((found.y >> k) & 1u) == 0u) {
                continue;
            }
            let bit = pack_bit(b - vec3<u32>(1u) + vec3<u32>(k % 3u, (k / 3u) % 3u, k / 9u), origin);
            fresh += 1u - (((*p).mask[bit >> 5u] >> (bit & 31u)) & 1u);
        }
        let full = (*p).quads + found.x > MAX_QUADS || (*p).cells + fresh > MAX_VERTICES;
        if ((*p).count > 0u && full) {
            break;
        }
        for (var k = 0u; k < 27u; k++) {
            if (((found.y >> k) & 1u) != 0u) {
                let bit = pack_bit(b - vec3<u32>(1u) + vec3<u32>(k % 3u, (k / 3u) % 3u, k / 9u), origin);
                (*p).mask[bit >> 5u] |= 1u << (bit & 31u);
            }
        }
        (*p).quads += found.x;
        (*p).cells += fresh;
        (*p).bricks[(*p).count] = (*p).next;
        (*p).count += 1u;
        (*p).next += 1u;
    }
    return (*p).count > 0u;
}

// The bit's position among the pack's set bits: the cell's index in its meshlet.
fn pack_rank(p: ptr<function, Pack>, bit: u32) -> u32 {
    var r = countOneBits((*p).mask[bit >> 5u] & ((1u << (bit & 31u)) - 1u));
    for (var w = 0u; w < (bit >> 5u); w++) {
        r += countOneBits((*p).mask[w]);
    }
    return r;
}

fn start_of(i: u32, r: Region) -> vec3<u32> {
    return r.base + vec3<u32>(i % r.size.x, (i / r.size.x) % r.size.y, i / (r.size.x * r.size.y));
}

// A cell's bit in a patch's mask; its cells start one sample before its starts.
fn cell_bit(c: vec3<u32>, r: Region) -> u32 {
    let l = c + vec3<u32>(1u) - r.base;
    let d = r.size + vec3<u32>(1u);
    return l.x + l.y * d.x + l.z * d.x * d.y;
}

fn bit_cell(bit: u32, r: Region) -> vec3<u32> {
    let d = r.size + vec3<u32>(1u);
    return r.base - vec3<u32>(1u) + vec3<u32>(bit % d.x, (bit / d.x) % d.y, bit / (d.x * d.y));
}

fn bits(r: Region) -> u32 {
    let d = r.size + vec3<u32>(1u);
    return d.x * d.y * d.z;
}

// A patch's quads, and in `mask` every cell they touch: at most 2 × 6 × 5 = 60 bits.
fn fill(r: Region, mask: ptr<function, array<u32, 4>>) -> u32 {
    for (var w = 0u; w < 4u; w++) {
        (*mask)[w] = 0u;
    }
    var quads = 0u;
    for (var i = 0u; i < r.size.x * r.size.y * r.size.z; i++) {
        let s = start_of(i, r);
        for (var axis = 0u; axis < 3u; axis++) {
            if (emits(s, axis).x == 0u) {
                continue;
            }
            quads += 1u;
            let cells = quad(s, axis);
            for (var k = 0u; k < 4u; k++) {
                let bit = cell_bit(cells[k], r);
                (*mask)[bit >> 5u] |= 1u << (bit & 31u);
            }
        }
    }
    return quads;
}

fn ones(mask: ptr<function, array<u32, 4>>) -> u32 {
    return countOneBits((*mask)[0]) + countOneBits((*mask)[1]) + countOneBits((*mask)[2])
        + countOneBits((*mask)[3]);
}

// The bit's position among the set bits: the cell's index in its meshlet.
fn rank(mask: ptr<function, array<u32, 4>>, bit: u32) -> u32 {
    var r = countOneBits((*mask)[bit >> 5u] & ((1u << (bit & 31u)) - 1u));
    for (var w = 0u; w < (bit >> 5u); w++) {
        r += countOneBits((*mask)[w]);
    }
    return r;
}

// Two triangles a quad, three bytes a triangle, each meshlet starting on a word.
fn quad_bytes(quads: u32) -> u32 {
    return (quads * 6u + 3u) & ~3u;
}

struct Plan {
    meshlets: u32,
    vertices: u32,
    bytes: u32,
}

// What region `q` takes of the pool: a region its packed meshlets, a patch one or none.
fn plan(q: u32) -> Plan {
    var out = Plan(0u, 0u, 0u);
    if (q < INNER) {
        var p: Pack;
        let origin = region(q).base;
        while (next_meshlet(origin, &p)) {
            out.meshlets += 1u;
            out.vertices += p.cells;
            out.bytes += quad_bytes(p.quads);
        }
        return out;
    }
    // Only a coarser leaf past it gives a +plane edge to the block.
    if (borrows == 0u) {
        return out;
    }
    var mask: array<u32, 4>;
    let quads = fill(region(q), &mask);
    if (quads != 0u) {
        out = Plan(1u, ones(&mask), quad_bytes(quads));
    }
    return out;
}
