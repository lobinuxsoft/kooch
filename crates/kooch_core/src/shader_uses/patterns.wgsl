// The engine's patterns: `// use: engine:patterns`. Every `///` function is a node in the graph.

/// Parallel bands, between `low` and `high`: brushed metal, scanlines, ripples.
fn lib_stripes(
    uv: vec2<f32>,
    direction: vec2<f32>, // @default(1, 0)
    frequency: f32, // @default(10)
    phase: f32,
    low: f32, // @default(0.4)
    high: f32, // @default(1)
) -> f32 {
    return mix(low, high, 0.5 + 0.5 * sin(dot(uv, direction) * frequency + phase));
}

/// A rainbow that shifts across `uv` and with `phase`: foil, soap film, oil.
fn lib_iridescence(
    uv: vec2<f32>,
    direction: vec2<f32>, // @default(1.8, 1.1)
    phase: f32,
) -> vec3<f32> {
    let hue = dot(uv, direction) + phase;
    return 0.5 + 0.5 * cos(6.2831855 * (hue + vec3<f32>(0.0, 0.33, 0.67)));
}

/// A soft band of light across `uv`, centred at `position` along `direction`.
fn lib_sheen(
    uv: vec2<f32>,
    direction: vec2<f32>, // @default(0.8, 0.6)
    position: f32, // @default(0.5)
    width: f32, // @default(0.15)
    strength: f32, // @default(1)
) -> f32 {
    let band = (dot(uv, direction) - position) / max(width, 0.00001);
    return strength * exp(-band * band);
}
