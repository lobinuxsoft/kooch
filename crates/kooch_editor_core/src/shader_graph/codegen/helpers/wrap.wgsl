// Wraps a lattice cell into `period`, so a noise hashed from it repeats every `period` cells and a
// uv that comes back to itself — a tiled texture, a full turn of polar coordinates — has no seam.
// An axis at 0 is left open, which is what every noise was before there was a period at all.
fn graph_wrap(cell: vec2<f32>, period: vec2<f32>) -> vec2<f32> {
    let wrapped = cell - period * floor(cell / max(period, vec2<f32>(1.0)));
    return select(cell, wrapped, period > vec2<f32>(0.5));
}

// The period one octave finer. Kept whole: a period that is not a whole number of cells does not
// wrap, so the octaves quantise rather than seam.
fn graph_wrap_octave(period: vec2<f32>, lacunarity: f32) -> vec2<f32> {
    return round(period * max(lacunarity, 1.0));
}

// The same wrap where the third axis is the phase, which turns rather than tiles.
fn graph_wrap3(cell: vec3<f32>, period: vec2<f32>) -> vec3<f32> {
    return vec3<f32>(graph_wrap(cell.xy, period), cell.z);
}
