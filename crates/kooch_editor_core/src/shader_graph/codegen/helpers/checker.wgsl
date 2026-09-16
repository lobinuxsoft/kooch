fn graph_checker(uv: vec2<f32>, tiles: vec2<f32>) -> f32 {
    let cell = floor(uv * tiles);
    return fract((cell.x + cell.y) * 0.5) * 2.0;
}
