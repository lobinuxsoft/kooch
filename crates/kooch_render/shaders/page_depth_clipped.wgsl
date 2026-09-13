// page_depth_clipped.wgsl — the same vertex, clipped to its page by the hardware instead of
// discarded by a fragment shader.

struct PageVertexClipped {
    @builtin(position) clip: vec4<f32>,
    // Four planes, one per edge of the page: left, right, bottom, top. Negative is outside, and the
    // clipper cuts the triangle where they cross zero. `maxClipDistances` is at least 8 wherever
    // the feature exists, so four is never the limit.
    @builtin(clip_distances) planes: array<f32, 4>,
}

@vertex
fn vs_page_clipped(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> PageVertexClipped {
    let geom = page_geometry(vertex_index, instance_index);
    var out: PageVertexClipped;
    out.clip = geom.clip;
    // 🔴 The page's own clip volume: in-page is `|local| <= w`, so each edge's signed distance is `w
    // ± local`.
    out.planes = array<f32, 4>(
        geom.w + geom.local.x,
        geom.w - geom.local.x,
        geom.w + geom.local.y,
        geom.w - geom.local.y,
    );
    // The tail of a short meshlet. The standard volume already rejects it — `page_geometry` sends
    // it to `(2, 2, 2, 1)` — but a plane the clipper is given has to agree, or a triangle survives
    // on the strength of the one test that was left unset.
    if geom.dead {
        out.planes = array<f32, 4>(-1.0, -1.0, -1.0, -1.0);
    }
    return out;
}
