// Pseudo-random numbers from a lattice point in three dimensions, for noise that moves in place.
// PCG3D (Jarzynski & Olano 2020), exact on the GPU and on a planet's CPU twin (#1930).
fn graph_pcg3(p: vec3<f32>) -> vec3<u32> {
    let a = bitcast<vec3<u32>>(vec3<i32>(p)) * 1664525u + 1013904223u;
    let b = graph_pcg3_mix(a);
    return graph_pcg3_mix(b ^ (b >> vec3<u32>(16u)));
}

fn graph_pcg3_mix(v: vec3<u32>) -> vec3<u32> {
    let x = v.x + v.y * v.z;
    let y = v.y + v.z * x;
    return vec3<u32>(x, y, v.z + x * y);
}

// The top 24 bits of each lane, which a float holds exactly, as 0..1.
fn graph_unit3(h: vec3<u32>) -> vec3<f32> {
    return vec3<f32>(h >> vec3<u32>(8u)) * 5.9604645e-8;
}

fn graph_hash3(p: vec3<f32>) -> f32 {
    return graph_unit3(graph_pcg3(p)).x;
}

fn graph_gradient3(p: vec3<f32>) -> vec3<f32> {
    return graph_unit3(graph_pcg3(p)) * 2.0 - 1.0;
}
