// Surface nets over one block's tile (#313), shared by the count and write passes. Both walk the
// same regions the same way, so the sizes one counts are the sizes the other fills.
//
// A cell holds a vertex when its eight samples change sign: the mean of its edge crossings. Each
// sign-changing edge whose start lies in the block gives a quad over the four cells around it, so
// two blocks of one level share their border cells and the mesh closes between them.
//
// Across a coarser leaf (#313 step 4c) a border cell is the coarse cell holding it, read from the
// coarse tile, and a sample on the shared border is the coarsest leaf's: every leaf there agrees on
// its sign.
//
// Meshlets are cut by the grid: the block's 16³ edge starts split into 4³ regions, one meshlet
// each, and a region too full for one splits into its eight 2³ eighths, which always fit.

override TILES_X: u32 = 32u;
override TILES_Y: u32 = 1u;

// The tile is 18 samples: the block's 16, and one either side. Cells sit between samples.
const TILE: u32 = 18u;
const CELLS: u32 = 17u;
const CELL_COUNT: u32 = 4913u;
const WORKGROUP: u32 = 256u;
const REGIONS: u32 = 64u;
const MAX_TRIANGLES: u32 = 124u;
const MAX_VERTICES: u32 = 64u;

@group(0) @binding(0) var atlas: texture_3d<f32>;

// Who holds the cells past each of the block's 26 neighbour directions; see `seam.rs`.
struct Near {
    cell: array<i32, 3>,
    sides: array<u32, 26>,
}

const SLOT_MASK: u32 = 0xffffffu;

var<workgroup> near: Near;
// Whether any side is a coarser leaf: most blocks have none.
var<workgroup> borrows: u32;

// Run by one invocation before the barrier that precedes any sample read.
fn load_near(n: Near) {
    near = n;
    borrows = 0u;
    for (var d = 0u; d < 26u; d++) {
        borrows |= select(0u, 1u, levels(n.sides[d]) != 0u);
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
    let d = select(vec3<i32>(0), vec3<i32>(-1), c == vec3<u32>(0u));
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

fn region_base(q: u32) -> vec3<u32> {
    return vec3<u32>(1u) + 4u * vec3<u32>(q % 4u, (q / 4u) % 4u, q / 16u);
}

fn eighth_base(base: vec3<u32>, e: u32) -> vec3<u32> {
    return base + 2u * vec3<u32>(e & 1u, (e >> 1u) & 1u, e >> 2u);
}

// A cell's bit in the mask of the region starting at `base`, whose cells start one sample before.
fn cell_bit(c: vec3<u32>, base: vec3<u32>, side: u32) -> u32 {
    let l = c + vec3<u32>(1u) - base;
    let d = side + 1u;
    return l.x + l.y * d + l.z * d * d;
}

fn bit_cell(bit: u32, base: vec3<u32>, side: u32) -> vec3<u32> {
    let d = side + 1u;
    return base - vec3<u32>(1u) + vec3<u32>(bit % d, (bit / d) % d, bit / (d * d));
}

// The region's quads, and in `mask` every cell they touch: at most 5³ = 125 bits.
fn fill(base: vec3<u32>, side: u32, mask: ptr<function, array<u32, 4>>) -> u32 {
    for (var w = 0u; w < 4u; w++) {
        (*mask)[w] = 0u;
    }
    var quads = 0u;
    for (var i = 0u; i < side * side * side; i++) {
        let s = base + vec3<u32>(i % side, (i / side) % side, i / (side * side));
        for (var axis = 0u; axis < 3u; axis++) {
            if (edge(s, axis).x == 0u) {
                continue;
            }
            quads += 1u;
            let cells = quad(s, axis);
            for (var k = 0u; k < 4u; k++) {
                let bit = cell_bit(cells[k], base, side);
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
    split: u32,
}

// How region `q` becomes meshlets: one, or one per non-empty eighth when one would not fit.
fn plan(q: u32) -> Plan {
    let base = region_base(q);
    var mask: array<u32, 4>;
    let quads = fill(base, 4u, &mask);
    if (quads == 0u) {
        return Plan(0u, 0u, 0u, 0u);
    }
    let cells = ones(&mask);
    if (quads * 2u <= MAX_TRIANGLES && cells <= MAX_VERTICES) {
        return Plan(1u, cells, quad_bytes(quads), 0u);
    }
    var p = Plan(0u, 0u, 0u, 1u);
    for (var e = 0u; e < 8u; e++) {
        let part = fill(eighth_base(base, e), 2u, &mask);
        if (part != 0u) {
            p.meshlets += 1u;
            p.vertices += ones(&mask);
            p.bytes += quad_bytes(part);
        }
    }
    return p;
}
