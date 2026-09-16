// Pseudo-random numbers from a cell coordinate, shared by every noise node.
fn graph_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.547);
}

fn graph_hash2(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.547);
}

// A random direction for a lattice point, each component in -1..1.
fn graph_gradient(cell: vec2<f32>) -> vec2<f32> {
    return graph_hash2(cell) * 2.0 - 1.0;
}
