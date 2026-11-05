fn graph_rainbow(phase: f32) -> vec4<f32> {
    return vec4<f32>(0.5 + 0.5 * cos(6.2831855 * (phase + vec3<f32>(0.0, 0.33, 0.67))), 1.0);
}
