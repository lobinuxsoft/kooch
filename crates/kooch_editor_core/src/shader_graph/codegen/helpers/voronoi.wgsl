// Cellular (Voronoi) noise: one point per cell, `randomness` of the way from the cell's middle to a
// random place, circling with `phase` so the cells can move.
struct GraphVoronoi {
    // Distance to the nearest point. Smoothness (0..1) rounds F1, F2 and the border alike.
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

fn graph_voronoi_point(cell: vec2<f32>, randomness: f32, phase: f32, period: vec2<f32>) -> vec2<f32> {
    let at = graph_wrap(cell, period);
    return mix(vec2<f32>(0.5), 0.5 + 0.5 * sin(phase + 6.2831855 * graph_hash2(at)), randomness);
}

// How far smoothness 1 blends: past it the 5x5 search is too narrow and the blend tears (measured).
const GRAPH_VORONOI_BLEND: f32 = 0.12;

// Smooth minimum over `distances` except `skipped`, around their true minimum `least`. Symmetric
// (log-sum-exp): a running polynomial blend depends on cell order and tears where cells swap.
fn graph_voronoi_blend(distances: ptr<function, array<f32, 25>>, skipped: i32, least: f32, k: f32) -> f32 {
    var sum = 0.0;
    for (var i = 0; i < 25; i = i + 1) {
        if i != skipped {
            sum = sum + exp(-((*distances)[i] - least) / k);
        }
    }
    return least - k * log(max(sum, 1e-6));
}

fn graph_voronoi(uv: vec2<f32>, randomness: f32, phase: f32, smoothness: f32, metric: i32, edges: bool, period: vec2<f32>) -> GraphVoronoi {
    let cell = floor(uv);
    let f = fract(uv);
    var out: GraphVoronoi;
    out.f1 = 8.0;
    out.f2 = 8.0;
    var distances: array<f32, 25>;
    var nearest_index = 0;
    var nearest = vec2<f32>(0.0);
    var nearest_cell = vec2<f32>(0.0);
    // 5x5, not 3x3: a random point two cells away is sometimes the second nearest, and 3x3 got F2
    // wrong often enough to speckle.
    for (var y = -2; y <= 2; y = y + 1) {
        for (var x = -2; x <= 2; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let point = graph_voronoi_point(cell + offset, randomness, phase, period);
            let r = offset + point - f;
            let d = graph_voronoi_distance(r, metric);
            let index = (y + 2) * 5 + x + 2;
            distances[index] = d;
            if d < out.f1 {
                out.f2 = out.f1;
                out.f1 = d;
                out.cell = graph_hash(graph_wrap(cell + offset, period));
                out.position = cell + offset + point;
                nearest = r;
                nearest_cell = offset;
                nearest_index = index;
            } else if d < out.f2 {
                out.f2 = d;
            }
        }
    }
    // The true border distance takes a second pass around the nearest cell — only when asked for.
    if edges {
        var edge = 8.0;
        for (var y = -2; y <= 2; y = y + 1) {
            for (var x = -2; x <= 2; x = x + 1) {
                let offset = nearest_cell + vec2<f32>(f32(x), f32(y));
                let r = offset + graph_voronoi_point(cell + offset, randomness, phase, period) - f;
                let apart = r - nearest;
                if dot(apart, apart) > 0.00001 {
                    edge = min(edge, dot(0.5 * (nearest + r), normalize(apart)));
                }
            }
        }
        out.edge = edge;
    }
    let blend = clamp(smoothness, 0.0, 1.0);
    if blend > 0.0 {
        let k = blend * GRAPH_VORONOI_BLEND;
        let f1 = graph_voronoi_blend(&distances, -1, out.f1, k);
        let f2 = graph_voronoi_blend(&distances, nearest_index, out.f2, k);
        // Bisector distances swap sets across a border, so their blend tears: half the blended gap
        // does not, and at full smoothness it takes over.
        out.edge = mix(out.edge, 0.5 * (f2 - f1), blend);
        out.f1 = f1;
        out.f2 = f2;
    }
    return out;
}
