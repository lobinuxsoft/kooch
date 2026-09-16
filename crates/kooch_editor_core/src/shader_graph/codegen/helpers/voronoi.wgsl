// Cellular (Voronoi) noise: one point per cell, `randomness` of the way from the cell's middle to a
// random place, circling with `phase` so the cells can move.
struct GraphVoronoi {
    // Distance to the nearest point — smoothed into its neighbours by `smoothness`.
    f1: f32,
    // Distance to the second nearest point.
    f2: f32,
    // Distance to the nearest cell border, which F2 - F1 only approximates.
    edge: f32,
    // A random value per cell, for colouring cells apart.
    cell: f32,
    // Where the nearest point is, in the scaled coordinate.
    position: vec2<f32>,
}

// 0 Euclidean (round cells), 1 Manhattan (diamonds), 2 Chebyshev (squares).
fn graph_voronoi_distance(d: vec2<f32>, metric: i32) -> f32 {
    let a = abs(d);
    return select(select(length(d), a.x + a.y, metric == 1), max(a.x, a.y), metric == 2);
}

fn graph_voronoi_point(cell: vec2<f32>, randomness: f32, phase: f32) -> vec2<f32> {
    return mix(vec2<f32>(0.5), 0.5 + 0.5 * sin(phase + 6.2831855 * graph_hash2(cell)), randomness);
}

fn graph_voronoi(uv: vec2<f32>, randomness: f32, phase: f32, smoothness: f32, metric: i32, edges: bool) -> GraphVoronoi {
    let cell = floor(uv);
    let f = fract(uv);
    var out: GraphVoronoi;
    out.f1 = 8.0;
    out.f2 = 8.0;
    // Smooth minimum, after Inigo Quilez's blended Voronoi: at zero smoothness it is F1 exactly.
    var blended = 8.0;
    let k = max(smoothness, 0.00001);
    var nearest = vec2<f32>(0.0);
    var nearest_cell = vec2<f32>(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let point = graph_voronoi_point(cell + offset, randomness, phase);
            let r = offset + point - f;
            let d = graph_voronoi_distance(r, metric);
            if d < out.f1 {
                out.f2 = out.f1;
                out.f1 = d;
                out.cell = graph_hash(cell + offset);
                out.position = cell + offset + point;
                nearest = r;
                nearest_cell = offset;
            } else if d < out.f2 {
                out.f2 = d;
            }
            let h = smoothstep(-1.0, 1.0, (blended - d) / k);
            blended = mix(blended, d, h) - h * (1.0 - h) * k / (1.0 + 3.0 * k);
        }
    }
    out.f1 = select(out.f1, blended, smoothness > 0.0);
    // The true border distance takes a second, wider pass around the nearest cell — only when asked for.
    if edges {
        var edge = 8.0;
        for (var y = -2; y <= 2; y = y + 1) {
            for (var x = -2; x <= 2; x = x + 1) {
                let offset = nearest_cell + vec2<f32>(f32(x), f32(y));
                let r = offset + graph_voronoi_point(cell + offset, randomness, phase) - f;
                let apart = r - nearest;
                if dot(apart, apart) > 0.00001 {
                    edge = min(edge, dot(0.5 * (nearest + r), normalize(apart)));
                }
            }
        }
        out.edge = edge;
    }
    return out;
}
