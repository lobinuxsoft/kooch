fn graph_sdf_fill(d: f32, softness: f32) -> f32 {
    return clamp(0.5 - d / max(softness, 0.00001), 0.0, 1.0);
}
