// Dual contouring over one block's tile (#313, #393), shared by the count and write passes. Both walk the
// same regions the same way, so the sizes one counts are the sizes the other fills.
//
// A cell holds a vertex when its eight samples change sign: dual contouring's QEF point (#393). Each
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
// Per region, a 64-bit set of the bricks its meshlets start at.
const CUT_WORDS: u32 = 16u;
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
// A leaf of our level, in this slot; see `SAME` in seam.rs.
const SAME: u32 = 0x40000000u;

// Which of the block's two meshes a job makes (#1999). The interior reads only the block's own
// samples, so it is made once per tile; the seam is the rest, made again when a neighbour changes.
const INTERIOR: u32 = 0u;
const SEAM: u32 = 1u;
var<workgroup> part: u32;

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
    return (side >> 24u) & 0x3fu;
}

fn is_same(side: u32) -> bool {
    return side != FINER && (side & SAME) != 0u;
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
    return shared_at(s, own(s));
}

// A sample one past the tile, from the tile of the leaf of our level across `d` whose cell reads
// it: a tile holds one sample either side, so it has every corner of its cells. Only where `owns`
// found that side `SAME`. Same-level tiles store a shared sample with the same bits.
fn past(s: vec3<u32>, d: vec3<i32>) -> f32 {
    let side = side_of(d);
    let base = textureLoad(atlas, tile_corner(side & SLOT_MASK) + vec3<i32>(s) - 16 * d, 0).x;
    return shared_at(s, base);
}

// A border sample from the coarsest leaf around it that has it as a sample, else `base`. Past the
// tile, the axis is the next leaf's interior: always across.
fn shared_at(s: vec3<u32>, base: f32) -> f32 {
    var best = 0u;
    var value = base;
    for (var m = 0u; m < 8u; m++) {
        var d = vec3<i32>(0);
        var around = true;
        for (var a = 0u; a < 3u; a++) {
            if (s[a] == TILE) {
                d[a] = 1;
                continue;
            }
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
        if (!around || all(d == vec3<i32>(0))) {
            continue;
        }
        let side = side_of(d);
        let up = levels(side);
        if (side == FINER || up <= best) {
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
    let d = outside(c);
    var v: array<f32, 8>;
    for (var k = 0u; k < 8u; k++) {
        let s = c + vec3<u32>(k & 1u, (k >> 1u) & 1u, k >> 2u);
        if (any(s == vec3<u32>(TILE))) {
            v[k] = past(s, d);
        } else {
            v[k] = at(s);
        }
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

// Gradient of the cell's trilinear interpolant at `p` in [0, 1]³: the Hermite normal of a crossing.
// From the eight corners alone, so two blocks sharing the cell agree on it bit for bit.
fn trilinear_gradient(v: array<f32, 8>, p: vec3<f32>) -> vec3<f32> {
    let q = vec3<f32>(1.0) - p;
    return vec3<f32>(
        q.y * q.z * (v[1] - v[0]) + p.y * q.z * (v[3] - v[2]) + q.y * p.z * (v[5] - v[4])
            + p.y * p.z * (v[7] - v[6]),
        q.x * q.z * (v[2] - v[0]) + p.x * q.z * (v[3] - v[1]) + q.x * p.z * (v[6] - v[4])
            + p.x * p.z * (v[7] - v[5]),
        q.x * q.y * (v[4] - v[0]) + p.x * q.y * (v[5] - v[1]) + q.x * p.y * (v[6] - v[2])
            + p.x * p.y * (v[7] - v[3]),
    );
}

// Pull toward the crossings' mean, per crossing: keeps a flat or rank-deficient QEF from flinging
// the vertex out, at a cost of under a hundredth of a voxel on a sharp corner.
const QEF_BIAS: f32 = 0.05;

// Dual contouring (#393): the point nearest every crossing's tangent plane, solved about the
// crossings' mean with a small pull toward it, then kept in the cell. The normal is the mean of the
// crossings'. A cell with no crossing, which only a finer neighbour borrows, takes one Newton step
// from its centre.
fn place(v: array<f32, 8>) -> array<vec3<f32>, 2> {
    // AᵀA (symmetric: six entries), Aᵀb and the crossings' sum, gathered in one pass: a per-thread
    // array indexed at run time would go to scratch (#1938).
    var xx = 0.0;
    var xy = 0.0;
    var xz = 0.0;
    var yy = 0.0;
    var yz = 0.0;
    var zz = 0.0;
    var atb = vec3<f32>(0.0);
    var mass = vec3<f32>(0.0);
    var normal = vec3<f32>(0.0);
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
            var p = vec3<f32>(f32(k & 1u), f32((k >> 1u) & 1u), f32(k >> 2u));
            p[axis] = a / (a - b);
            let grad = trilinear_gradient(v, p);
            let n = select(vec3<f32>(0.0), normalize(grad), dot(grad, grad) > 0.0);
            xx += n.x * n.x;
            xy += n.x * n.y;
            xz += n.x * n.z;
            yy += n.y * n.y;
            yz += n.y * n.z;
            zz += n.z * n.z;
            atb += n * dot(n, p);
            mass += p;
            normal += n;
            count += 1.0;
        }
    }
    let g = vec3<f32>(
        (v[1] + v[3] + v[5] + v[7]) - (v[0] + v[2] + v[4] + v[6]),
        (v[2] + v[3] + v[6] + v[7]) - (v[0] + v[1] + v[4] + v[5]),
        (v[4] + v[5] + v[6] + v[7]) - (v[0] + v[1] + v[2] + v[3]),
    );
    let still = !(dot(g, g) > 0.0);
    let fallback = select(normalize(g), vec3<f32>(0.0, 1.0, 0.0), still);
    if (count == 0.0) {
        var mean = 0.0;
        for (var k = 0u; k < 8u; k++) {
            mean += v[k] * 0.125;
        }
        // g is four times the gradient across the cell.
        let newton = select(g * (4.0 * mean / dot(g, g)), vec3<f32>(0.0), still);
        let inside = clamp(vec3<f32>(0.5) - newton, vec3<f32>(0.0), vec3<f32>(1.0));
        return array<vec3<f32>, 2>(inside, fallback);
    }
    mass /= count;
    let ata = mat3x3<f32>(vec3<f32>(xx, xy, xz), vec3<f32>(xy, yy, yz), vec3<f32>(xz, yz, zz));
    // Solved about the mean: (AᵀA + λI) y = Aᵀb − AᵀA·mass.
    let rhs = atb - ata * mass;
    let m = ata + mat3x3<f32>(
        vec3<f32>(QEF_BIAS * count, 0.0, 0.0),
        vec3<f32>(0.0, QEF_BIAS * count, 0.0),
        vec3<f32>(0.0, 0.0, QEF_BIAS * count),
    );
    // Positive definite through the bias; the guard only catches a NaN sample.
    let det = determinant(m);
    var y = vec3<f32>(0.0);
    if (det > 1e-12) {
        // Cofactors: for a symmetric matrix, its inverse times the determinant.
        let adj = mat3x3<f32>(cross(m[1], m[2]), cross(m[2], m[0]), cross(m[0], m[1]));
        y = (adj * rhs) / det;
    }
    let inside = clamp(mass + y, vec3<f32>(0.0), vec3<f32>(1.0));
    let up = select(normalize(normal), fallback, !(dot(normal, normal) > 0.0));
    return array<vec3<f32>, 2>(inside, up);
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

// Whether the block emits the edge from `s` along `axis`. None whose cells reach a finer leaf,
// which emits it itself. One whose start is past a + plane belongs to the leaf holding that cell:
// if it is coarser, to the first block of our level holding one of the other cells, in an order
// every block shares (#393). That block reads the cells past its tile from the leaves there.
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
        // A cell past a + plane of a leaf of our level is read from it, so it must be in hand.
        let unread = any(d == vec3<i32>(1)) && levels(side) == 0u && !is_same(side);
        if (side == FINER || unread) {
            return false;
        }
    }
    let start = outside(s);
    if (all(start == vec3<i32>(0))) {
        return true;
    }
    if (levels(side_of(start)) == 0u) {
        return false;
    }
    for (var k = 0u; k < 4u; k++) {
        if (k == 2u) {
            continue;
        }
        let d = outside(cells[k]);
        if (all(d == vec3<i32>(0))) {
            return true;
        }
        if (levels(side_of(d)) == 0u) {
            return false;
        }
    }
    return false;
}

// Whether no sample the edge's quad reads is a border one: its samples, and its four cells'
// corners, lie in 2..16, where `at` reads the block's own tile and `cell_vertex` places in it.
fn inner_edge(s: vec3<u32>, axis: u32) -> bool {
    var lo = vec3<u32>(3u);
    lo[axis] = 2u;
    return all(s >= lo) && all(s <= vec3<u32>(15u));
}

// Whether cell `c` can hold a vertex of the job's part: the interior's cells are 2..15, the seam's
// every cell with a coordinate outside 3..14. Cells 2 and 15 are in both, placed alike by each.
fn in_part(c: vec3<u32>) -> bool {
    if (part == INTERIOR) {
        return all(c >= vec3<u32>(2u)) && all(c <= vec3<u32>(15u));
    }
    return any(c < vec3<u32>(3u)) || any(c > vec3<u32>(14u));
}

// `edge` over the edges the block emits, of the job's part.
fn emits(s: vec3<u32>, axis: u32) -> vec2<u32> {
    if (inner_edge(s, axis) != (part == INTERIOR) || !owns(s, axis)) {
        return vec2<u32>(0u);
    }
    return edge(s, axis);
}

// Whether cell `c` takes a vertex. Cell 17's samples are past the tile, so it takes one only when an
// emitted edge reaches it.
fn has_vertex(c: vec3<u32>) -> bool {
    if (!in_part(c)) {
        return false;
    }
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

// What region `q` takes of the pool: a region its packed meshlets, a patch one or none. A region
// also sets in `cuts` the brick each of its meshlets starts at.
fn plan(q: u32, cuts: ptr<function, vec2<u32>>) -> Plan {
    var out = Plan(0u, 0u, 0u);
    if (q < INNER) {
        var p = Pack(0u, 0u, 0u, 0u);
        let origin = region(q).base;
        while (next_meshlet(origin, &p, q)) {
            (*cuts)[p.first >> 5u] |= 1u << (p.first & 31u);
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
