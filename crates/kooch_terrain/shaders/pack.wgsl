// Meshlet packing over a block's 2³ bricks (#1935, #1938), appended to nets.wgsl. The count pass
// packs; the write pass rebuilds each meshlet from the bricks the count's cuts give it.

// Per 2³ brick of the block's 16³ starts, the edges it emits: bit `axis * 8 + start`, and from bit
// 24 which of its starts are inside. Found once,
// so packing a brick reads one word instead of 24 (#1935).
var<workgroup> edges: array<atomic<u32>, 512>;

fn clear_edges(lid: u32) {
    for (var w = lid; w < 512u; w += WORKGROUP) {
        atomicStore(&edges[w], 0u);
    }
}

// The brick holding start `s`, and the start's bit within it before the axis offset.
fn edge_bit(s: vec3<u32>) -> vec2<u32> {
    let l = s - vec3<u32>(1u);
    let b = l >> vec3<u32>(1u);
    let i = l & vec3<u32>(1u);
    return vec2<u32>(b.x + b.y * 8u + b.z * 64u, i.x | (i.y << 1u) | (i.z << 2u));
}

fn mark_edges(lid: u32) {
    for (var i = lid; i < 4096u; i += WORKGROUP) {
        let s = vec3<u32>(1u) + vec3<u32>(i % 16u, (i / 16u) % 16u, i / 256u);
        let at = edge_bit(s);
        var word = 0u;
        for (var axis = 0u; axis < 3u; axis++) {
            let found = emits(s, axis);
            word |= select(0u, 1u << (axis * 8u + at.y), found.x != 0u);
            // Whether the start is inside, the same for every axis: the write reads it here.
            word |= select(0u, 1u << (24u + at.y), found.x != 0u && found.y != 0u);
        }
        if (word != 0u) {
            atomicOr(&edges[at.x], word);
        }
    }
}

// The edges of the brick based at `b`.
fn brick_edges(b: vec3<u32>) -> u32 {
    return atomicLoad(&edges[edge_bit(b).x]) & 0xffffffu;
}

// Per start of the brick at `b`, whether it lies inside, for the starts that emit an edge.
fn brick_inside(b: vec3<u32>) -> u32 {
    return atomicLoad(&edges[edge_bit(b).x]) >> 24u;
}

// An edge of a brick's word: its start and axis.
fn brick_edge(b: vec3<u32>, bit: u32) -> vec4<u32> {
    let i = bit & 7u;
    return vec4<u32>(b + vec3<u32>(i & 1u, (i >> 1u) & 1u, i >> 2u), bit >> 3u);
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
    var word = brick_edges(b);
    let quads = countOneBits(word);
    var cells = 0u;
    while (word != 0u) {
        let bit = countTrailingZeros(word);
        word &= word - 1u;
        // The quad's cells are a 2×2 square from `l + 1 - u - v`, as bits of the 3³ (strides 1, 3, 9).
        let l = vec3<u32>(bit & 1u, (bit >> 1u) & 1u, (bit >> 2u) & 1u);
        let axis = bit >> 3u;
        var corner = l + vec3<u32>(1u);
        corner[(axis + 1u) % 3u] -= 1u;
        corner[(axis + 2u) % 3u] -= 1u;
        // Per axis, the four cells as bits from the lowest: strides u and v of 1, 3, 9.
        let square = select(select(27u, 1539u, axis == 1u), 4617u, axis == 0u);
        cells |= square << (corner.x + 3u * corner.y + 9u * corner.z);
    }
    return vec2<u32>(quads, cells);
}

// Per brick, its quads (5 bits) and the 3³ cells they touch (27): found by every thread at once,
// so a packing chain reads a word a brick (#1938).
var<workgroup> brick_sizes: array<u32, 512>;

// Run after `mark_edges` and its barrier, and needs one after.
fn mark_bricks(lid: u32) {
    for (var i = lid; i < 512u; i += WORKGROUP) {
        let found = brick(vec3<u32>(1u) + 2u * vec3<u32>(i % 8u, (i / 8u) % 8u, i / 64u));
        brick_sizes[i] = found.x | (found.y << 5u);
    }
}

fn brick_size(b: vec3<u32>) -> vec2<u32> {
    let word = brick_sizes[edge_bit(b).x];
    return vec2<u32>(word & 31u, word >> 5u);
}

// A cell's bit among the 9³ cells of the region at `origin`.
fn pack_bit(c: vec3<u32>, origin: vec3<u32>) -> u32 {
    let l = c + vec3<u32>(1u) - origin;
    return l.x + 9u * l.y + 81u * l.z;
}

// A meshlet's cells, 9³ bits of its region. In workgroup memory, one slot a packing thread: a
// thread's own array indexed at run time goes to scratch, ~0.25 ms a pass (#1938).
alias Mask = array<u32, PACK_WORDS>;
const MASK_SLOTS: u32 = 64u;
var<workgroup> masks: array<Mask, MASK_SLOTS>;

// One meshlet of a region: bricks `first..next` in Morton order, and its counts. An empty brick in
// the range has no edges, so walking it costs a load.
struct Pack {
    first: u32,
    next: u32,
    quads: u32,
    cells: u32,
}

// Packs the next meshlet of the region at `origin` into `p` and `mask`; false once its bricks are
// spent. The count pass alone packs: the write pass reads where each meshlet starts.
fn next_meshlet(origin: vec3<u32>, p: ptr<function, Pack>, slot: u32) -> bool {
    (*p).first = (*p).next;
    (*p).quads = 0u;
    (*p).cells = 0u;
    for (var w = 0u; w < PACK_WORDS; w++) {
        masks[slot][w] = 0u;
    }
    while ((*p).next < 64u) {
        let b = brick_base(origin, (*p).next);
        let found = brick_size(b);
        if (found.x == 0u) {
            (*p).next += 1u;
            continue;
        }
        // Bricks are 2 apart and 3 cells wide, so x runs of a brick are 3 adjacent bits of the mask.
        let row0 = b - origin;
        var fresh = 0u;
        for (var r = 0u; r < 9u; r++) {
            fresh += pack_row(slot, row0, r, found.y, false);
        }
        let full = (*p).quads + found.x > MAX_QUADS || (*p).cells + fresh > MAX_VERTICES;
        if ((*p).quads > 0u && full) {
            break;
        }
        for (var r = 0u; r < 9u; r++) {
            pack_row(slot, row0, r, found.y, true);
        }
        (*p).quads += found.x;
        (*p).cells += fresh;
        (*p).next += 1u;
    }
    return (*p).quads > 0u;
}

// Adds brick `m` of the region at `origin` to mask `slot`; its quads.
fn add_brick(origin: vec3<u32>, m: u32, slot: u32) -> u32 {
    let b = brick_base(origin, m);
    let found = brick_size(b);
    if (found.x == 0u) {
        return 0u;
    }
    for (var r = 0u; r < 9u; r++) {
        pack_row(slot, b - origin, r, found.y, true);
    }
    return found.x;
}

// Row `r` (y + 3z) of a brick's cells against mask `slot`: the cells not in it yet, and with `add`
// adds them.
fn pack_row(slot: u32, row0: vec3<u32>, r: u32, cells: u32, add: bool) -> u32 {
    let run = (cells >> (3u * r)) & 7u;
    if (run == 0u) {
        return 0u;
    }
    let g = row0.x + 9u * (row0.y + r % 3u) + 81u * (row0.z + r / 3u);
    let w = g >> 5u;
    let o = g & 31u;
    let lo = run << o;
    var fresh = countOneBits(lo & ~masks[slot][w]);
    if (add) {
        masks[slot][w] |= lo;
    }
    if (o > 29u) {
        let hi = run >> (32u - o);
        fresh += countOneBits(hi & ~masks[slot][w + 1u]);
        if (add) {
            masks[slot][w + 1u] |= hi;
        }
    }
    return fresh;
}
