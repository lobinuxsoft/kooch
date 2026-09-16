// Cellular (Voronoi) noise: one random point per cell, `jitter` of the way from the cell's corner.
// x is the distance to the nearest point (F1), y to the second nearest (F2), z is F2 - F1 — the cell
// borders — and w a random value per cell, for colouring cells apart.
fn graph_voronoi(uv: vec2<f32>, jitter: f32) -> vec4<f32> {
    let cell = floor(uv);
    let f = fract(uv);
    var first = 8.0;
    var second = 8.0;
    var id = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let point = offset + graph_hash2(cell + offset) * jitter - f;
            let distance = length(point);
            if distance < first {
                second = first;
                first = distance;
                id = graph_hash(cell + offset);
            } else if distance < second {
                second = distance;
            }
        }
    }
    return vec4<f32>(first, second, second - first, id);
}
