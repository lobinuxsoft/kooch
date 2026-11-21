// Pseudo-random numbers from a cell coordinate, shared by every noise node. PCG2D (Jarzynski & Olano
// 2020) on the integer cell: integer ops are exact on the GPU and on a planet's CPU twin, where `sin`
// of a large argument is not (#1930).
fn graph_pcg2(p: vec2<f32>) -> vec2<u32> {
    let a = bitcast<vec2<u32>>(vec2<i32>(p)) * 1664525u + 1013904223u;
    let b = graph_pcg2_mix(a);
    let c = graph_pcg2_mix(b ^ (b >> vec2<u32>(16u)));
    return c ^ (c >> vec2<u32>(16u));
}

fn graph_pcg2_mix(v: vec2<u32>) -> vec2<u32> {
    let x = v.x + v.y * 1664525u;
    return vec2<u32>(x, v.y + x * 1664525u);
}

// The top 24 bits, which a float holds exactly, as 0..1.
fn graph_unit(h: u32) -> f32 {
    return f32(h >> 8u) * 5.9604645e-8;
}

fn graph_hash(p: vec2<f32>) -> f32 {
    let h = graph_pcg2(p);
    // Not `h.x` alone: Voronoi colours a cell by this and places its point by `graph_hash2`.
    return graph_unit(h.x ^ h.y);
}

fn graph_hash2(p: vec2<f32>) -> vec2<f32> {
    let h = graph_pcg2(p);
    return vec2<f32>(graph_unit(h.x), graph_unit(h.y));
}

// A random direction for a lattice point, each component in -1..1.
fn graph_gradient(cell: vec2<f32>) -> vec2<f32> {
    return graph_hash2(cell) * 2.0 - 1.0;
}
