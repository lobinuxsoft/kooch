// Planet meshing (#313 step 4a): surface nets over one block's tile per workgroup.
//
// A cell holds a vertex when its eight samples change sign: the mean of its edge crossings. Each
// sign-changing edge whose start lies in the block gives a quad over the four cells around it, so
// two blocks of one level share their border cells and the mesh closes between them. Output is
// compacted into shared buffers; `header` says where each block's part starts.

override TILES_X: u32 = 32u;
override TILES_Y: u32 = 1u;
override VERTEX_CAP: u32 = 65536u;
override INDEX_CAP: u32 = 393216u;

// The tile is 18 samples: the block's 16, and one either side. Cells sit between samples.
const TILE: u32 = 18u;
const CELLS: u32 = 17u;
const CELL_COUNT: u32 = 4913u;
const MAP_WORDS: u32 = 2457u;
const WORKGROUP: u32 = 256u;
const NONE: u32 = 0xffffffffu;

@group(0) @binding(0) var atlas: texture_3d<f32>;
@group(0) @binding(1) var<storage, read> jobs: array<u32>;
// [vertex total, index total], then per block: first vertex, vertex count, first index, index count.
@group(0) @binding(2) var<storage, read_write> header: array<atomic<u32>>;
// Per vertex: position in voxels from the block's corner, then the outward normal.
@group(0) @binding(3) var<storage, read_write> vertices: array<f32>;
@group(0) @binding(4) var<storage, read_write> indices: array<u32>;

// Cell → its vertex + 1, two 16-bit entries per word; 0 is no vertex.
var<workgroup> map: array<atomic<u32>, MAP_WORDS>;
var<workgroup> counter: atomic<u32>;
var<workgroup> base: u32;
var<workgroup> failed: u32;
var<workgroup> corner: vec3<i32>;

fn at(s: vec3<u32>) -> f32 {
    return textureLoad(atlas, corner + vec3<i32>(s), 0).x;
}

fn cell_of(i: u32) -> vec3<u32> {
    return vec3<u32>(i % CELLS, (i / CELLS) % CELLS, i / (CELLS * CELLS));
}

fn cell_index(c: vec3<u32>) -> u32 {
    return c.x + c.y * CELLS + c.z * CELLS * CELLS;
}

fn vertex_of(i: u32) -> u32 {
    let word = atomicLoad(&map[i >> 1u]);
    return (word >> ((i & 1u) * 16u)) & 0xffffu;
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

// The mean of the cell's edge crossings, within the cell, and the gradient as its normal.
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
    let normal = select(vec3<f32>(0.0, 1.0, 0.0), normalize(g), dot(g, g) > 0.0);
    return array<vec3<f32>, 2>(sum / max(count, 1.0), normal);
}

// Whether the edge from `s` along `axis` changes sign, and if so whether it starts inside.
fn edge(s: vec3<u32>, axis: u32) -> vec2<u32> {
    var e = vec3<u32>(0u);
    e[axis] = 1u;
    let a = at(s);
    let b = at(s + e);
    return vec2<u32>(select(0u, 1u, (a < 0.0) != (b < 0.0)), select(0u, 1u, a < 0.0));
}

// Reserves `count` from `total` for this block, or marks the block failed if it does not fit.
fn reserve(total: u32, count: u32, cap: u32, entry: u32) {
    let start = atomicAdd(&header[total], count);
    base = start;
    if (start + count > cap) {
        failed = 1u;
        atomicStore(&header[entry], NONE);
    } else {
        atomicStore(&header[entry], start);
    }
    atomicStore(&header[entry + 1u], count);
}

@compute @workgroup_size(256)
fn mesh_main(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let block = wid.x;
    let slot = jobs[block];
    let entry = 2u + block * 4u;
    if (lid == 0u) {
        corner = vec3<i32>(
            vec3<u32>(slot % TILES_X, (slot / TILES_X) % TILES_Y, slot / (TILES_X * TILES_Y)) * TILE,
        );
        atomicStore(&counter, 0u);
        failed = 0u;
    }
    for (var i = lid; i < MAP_WORDS; i += WORKGROUP) {
        atomicStore(&map[i], 0u);
    }
    workgroupBarrier();

    // Which cells hold a vertex, and its number within the block.
    for (var i = lid; i < CELL_COUNT; i += WORKGROUP) {
        if (crosses(corners(cell_of(i)))) {
            let n = atomicAdd(&counter, 1u) + 1u;
            atomicOr(&map[i >> 1u], n << ((i & 1u) * 16u));
        }
    }
    workgroupBarrier();
    if (lid == 0u) {
        reserve(0u, atomicLoad(&counter), VERTEX_CAP, entry);
    }
    if (workgroupUniformLoad(&failed) != 0u) {
        return;
    }
    let first_vertex = workgroupUniformLoad(&base);
    for (var i = lid; i < CELL_COUNT; i += WORKGROUP) {
        let n = vertex_of(i);
        if (n == 0u) {
            continue;
        }
        let c = cell_of(i);
        let placed = place(corners(c));
        // Sample 1 is the block's corner.
        let p = vec3<f32>(c) + placed[0] - vec3<f32>(1.0);
        let o = (first_vertex + n - 1u) * 6u;
        vertices[o] = p.x;
        vertices[o + 1u] = p.y;
        vertices[o + 2u] = p.z;
        vertices[o + 3u] = placed[1].x;
        vertices[o + 4u] = placed[1].y;
        vertices[o + 5u] = placed[1].z;
    }

    // The block's own edges: those starting at samples 1..=16 on every axis.
    if (lid == 0u) {
        atomicStore(&counter, 0u);
    }
    workgroupBarrier();
    for (var i = lid; i < 4096u; i += WORKGROUP) {
        let s = vec3<u32>(i % 16u, (i / 16u) % 16u, i / 256u) + vec3<u32>(1u);
        for (var axis = 0u; axis < 3u; axis++) {
            if (edge(s, axis).x != 0u) {
                atomicAdd(&counter, 1u);
            }
        }
    }
    workgroupBarrier();
    if (lid == 0u) {
        reserve(1u, atomicLoad(&counter) * 6u, INDEX_CAP, entry + 2u);
        atomicStore(&counter, 0u);
    }
    if (workgroupUniformLoad(&failed) != 0u) {
        return;
    }
    let first_index = workgroupUniformLoad(&base);
    for (var i = lid; i < 4096u; i += WORKGROUP) {
        let s = vec3<u32>(i % 16u, (i / 16u) % 16u, i / 256u) + vec3<u32>(1u);
        for (var axis = 0u; axis < 3u; axis++) {
            let found = edge(s, axis);
            if (found.x == 0u) {
                continue;
            }
            // u × v = axis, so q0 q1 q2 q3 turn counter-clockwise seen from +axis.
            var u = vec3<u32>(0u);
            var v = vec3<u32>(0u);
            u[(axis + 1u) % 3u] = 1u;
            v[(axis + 2u) % 3u] = 1u;
            let q0 = vertex_of(cell_index(s - u - v)) - 1u;
            let q1 = vertex_of(cell_index(s - v)) - 1u;
            let q2 = vertex_of(cell_index(s)) - 1u;
            let q3 = vertex_of(cell_index(s - u)) - 1u;
            let o = first_index + atomicAdd(&counter, 1u) * 6u;
            // Starting inside, the surface faces +axis; outside, the other way.
            if (found.y != 0u) {
                indices[o] = q0;
                indices[o + 1u] = q1;
                indices[o + 2u] = q2;
                indices[o + 3u] = q0;
                indices[o + 4u] = q2;
                indices[o + 5u] = q3;
            } else {
                indices[o] = q0;
                indices[o + 1u] = q2;
                indices[o + 2u] = q1;
                indices[o + 3u] = q0;
                indices[o + 4u] = q3;
                indices[o + 5u] = q2;
            }
        }
    }
}
