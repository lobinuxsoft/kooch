// Planckian locus (Kang et al. 2002) to linear sRGB, normalised so the brightest channel is 1:
// the hue of the glow, its brightness left to the caller.
fn graph_blackbody(kelvin: f32) -> vec4<f32> {
    let t = clamp(kelvin, 1667.0, 25000.0);
    let i = 1.0 / t;
    var x: f32;
    if (t < 4000.0) {
        x = ((-0.2661239e9 * i - 0.2343589e6) * i + 0.8776956e3) * i + 0.179910;
    } else {
        x = ((-3.0258469e9 * i + 2.1070379e6) * i + 0.2226347e3) * i + 0.240390;
    }
    var y: f32;
    if (t < 2222.0) {
        y = ((-1.1063814 * x - 1.34811020) * x + 2.18555832) * x - 0.20219683;
    } else if (t < 4000.0) {
        y = ((-0.9549476 * x - 1.37418593) * x + 2.09137015) * x - 0.16748867;
    } else {
        y = ((3.0817580 * x - 5.87338670) * x + 3.75112997) * x - 0.37001483;
    }
    let xyz = vec3<f32>(x / y, 1.0, (1.0 - x - y) / y);
    let rgb = max(vec3<f32>(
        dot(xyz, vec3<f32>(3.2404542, -1.5371385, -0.4985314)),
        dot(xyz, vec3<f32>(-0.9692660, 1.8760108, 0.0415560)),
        dot(xyz, vec3<f32>(0.0556434, -0.2040259, 1.0572252)),
    ), vec3<f32>(0.0));
    return vec4<f32>(rgb / max(max(rgb.r, rgb.g), max(rgb.b, 0.00001)), 1.0);
}
