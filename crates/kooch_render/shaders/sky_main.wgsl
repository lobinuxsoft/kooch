// sky_main.wgsl — procedural sky with volumetric clouds.
//
// Full-screen triangle. For each pixel:
//   1. Reconstruct a world-space view ray direction (same math as raymarch).
//   2. Compute a vertical gradient sky_color (horizon → zenith) from the
//      ray's Y component.
//   3. If `cloud_coverage > 0` and the ray intersects the cloud slab,
//      ray-march the slab accumulating density (hash value noise + FBM)
//      and single-scattering towards the sun (Henyey-Greenstein phase +
//      a short light march).
//   4. Composite: `final = mix(sky_color, cloud_color, cloud_alpha)`.
//
// Runs BEFORE the ray-march pass when a SkyRenderer entity is active.
// Clears the target and writes `frag_depth = 1.0`.

struct CameraUniforms {
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    inverse_view: mat4x4<f32>,
    inverse_projection: mat4x4<f32>,
    position: vec3<f32>,
    _pad0: f32,
}

struct SkyUniforms {
    top_color: vec4<f32>,
    bottom_color: vec4<f32>,
    sun_direction: vec4<f32>,   // xyz = normalized dir, w unused
    sun_color: vec4<f32>,
    // x = cloud_coverage [0,1], y = cloud_density, z = cloud_height, w = cloud_thickness
    cloud_params: vec4<f32>,
    // xyz = wind velocity (wind_dir * wind_speed), w = time seconds
    wind_time: vec4<f32>,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> sky: SkyUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle — no vertex buffer needed.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var out: VertexOutput;
    out.position = vec4<f32>(positions[vi], 0.0, 1.0);
    out.uv = uvs[vi];
    return out;
}

// Reconstructs the world-space view direction for UV in [0, 1].
// Mirrors `generate_ray` in raymarch_main.wgsl.
fn view_direction(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let near_h = camera.inverse_projection * vec4<f32>(ndc, -1.0, 1.0);
    let far_h  = camera.inverse_projection * vec4<f32>(ndc,  1.0, 1.0);
    let near_view = near_h.xyz / near_h.w;
    let far_view  = far_h.xyz  / far_h.w;
    let near_world = (camera.inverse_view * vec4<f32>(near_view, 1.0)).xyz;
    let far_world  = (camera.inverse_view * vec4<f32>(far_view,  1.0)).xyz;
    return normalize(far_world - near_world);
}

// ---- Noise -----------------------------------------------------------------

// 3D hash → [0, 1). Cheap, deterministic, good enough for FBM value noise.
fn hash13(p: vec3<f32>) -> f32 {
    var x = fract(p * 0.3183099 + 0.1);
    x = x * 17.0;
    return fract(x.x * x.y * x.z * (x.x + x.y + x.z));
}

// Value noise — trilinear interpolation of 8 corner hashes. Smoother than
// gradient noise to compute and plenty for low-frequency cloud shape.
fn value_noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    // Quintic Hermite smoothing for C² continuity — removes grid artifacts.
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);

    let n000 = hash13(i + vec3<f32>(0.0, 0.0, 0.0));
    let n100 = hash13(i + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = hash13(i + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = hash13(i + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = hash13(i + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = hash13(i + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = hash13(i + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = hash13(i + vec3<f32>(1.0, 1.0, 1.0));

    let nx00 = mix(n000, n100, u.x);
    let nx10 = mix(n010, n110, u.x);
    let nx01 = mix(n001, n101, u.x);
    let nx11 = mix(n011, n111, u.x);

    let nxy0 = mix(nx00, nx10, u.y);
    let nxy1 = mix(nx01, nx11, u.y);

    return mix(nxy0, nxy1, u.z);
}

// 4-octave fBm — low-cost cumulus shape.
fn fbm4(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    for (var i = 0; i < 4; i = i + 1) {
        sum = sum + amp * value_noise3(p * freq);
        freq = freq * 2.03; // slight non-integer to break aliasing
        amp = amp * 0.5;
    }
    return sum;
}

// ---- Cloud density ---------------------------------------------------------

// Returns density in [0, 1] for a world-space point inside the cloud slab.
// Shaped by `cloud_coverage` (cuts off low density) and `cloud_density`
// (overall multiplier after shaping). Returns 0 outside the slab.
fn cloud_density(p_world: vec3<f32>, time_sec: f32) -> f32 {
    let coverage  = sky.cloud_params.x;
    let density   = sky.cloud_params.y;
    let cloud_lo  = sky.cloud_params.z;
    let cloud_hi  = sky.cloud_params.z + sky.cloud_params.w;
    let y = p_world.y;
    if (y < cloud_lo || y > cloud_hi) {
        return 0.0;
    }
    // Vertical profile — fade density at top and bottom so the slab looks
    // like puffy cumulus instead of a hard rectangle.
    let h = (y - cloud_lo) / max(sky.cloud_params.w, 0.001);
    let vertical = smoothstep(0.0, 0.2, h) * smoothstep(1.0, 0.6, h);

    // Scroll the noise with wind.
    let wind = sky.wind_time.xyz * time_sec;
    let noise_pos = (p_world + wind) * 0.012;

    // Base shape (low-frequency) + detail erosion (high-frequency).
    let base = fbm4(noise_pos);
    let detail = fbm4(noise_pos * 6.0);
    let shape = base - detail * 0.25;

    // Coverage cut — below threshold → no cloud, above → ramp up.
    let d = smoothstep(1.0 - coverage, 1.0 - coverage * 0.5, shape) * vertical;
    return clamp(d * density, 0.0, 1.0);
}

// ---- Ray / slab intersection ----------------------------------------------

// Returns (t_enter, t_exit, hit). `t_enter < 0` when the camera is inside
// the slab — callers should max with 0. `hit == false` when the ray misses
// the slab entirely (parallel ray above/below the slab).
struct SlabHit {
    t_enter: f32,
    t_exit: f32,
    hit: bool,
}

fn ray_slab(origin: vec3<f32>, dir: vec3<f32>) -> SlabHit {
    let y_lo = sky.cloud_params.z;
    let y_hi = sky.cloud_params.z + sky.cloud_params.w;
    var out: SlabHit;
    out.hit = false;
    out.t_enter = 0.0;
    out.t_exit = 0.0;

    // Parallel ray: in or out of slab by origin.y.
    if (abs(dir.y) < 1e-4) {
        if (origin.y >= y_lo && origin.y <= y_hi) {
            out.hit = true;
            out.t_enter = 0.0;
            out.t_exit = 20000.0; // a long distance
        }
        return out;
    }

    let t1 = (y_lo - origin.y) / dir.y;
    let t2 = (y_hi - origin.y) / dir.y;
    out.t_enter = min(t1, t2);
    out.t_exit  = max(t1, t2);
    if (out.t_exit < 0.0) {
        return out; // slab entirely behind camera
    }
    out.hit = true;
    return out;
}

// ---- Lighting --------------------------------------------------------------

// Henyey-Greenstein phase function. `g` controls anisotropy:
// g ≈ 0.3..0.8 → forward scatter (cloud halo around the sun).
fn phase_hg(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = pow(1.0 + g2 - 2.0 * g * cos_theta, 1.5);
    return (1.0 - g2) / (4.0 * 3.14159265 * max(denom, 1e-4));
}

// Short light march toward the sun. Returns transmittance along the ray
// to approximate how much sunlight reaches `p`.
//
// 3 steps × 10 world units = 30-unit reach — enough for self-shadowing in
// cumulus-scale clouds without paying for rays that leave the slab
// anyway. Reducing from 4→3 steps saves ~25% of the light-march cost
// across every sample the primary march takes.
fn light_transmittance(p: vec3<f32>, sun_dir: vec3<f32>, time_sec: f32) -> f32 {
    let light_steps = 3;
    let light_step_size = 10.0; // world units
    var t_light = 0.0;
    var acc = 0.0;
    for (var i = 0; i < light_steps; i = i + 1) {
        t_light = t_light + light_step_size;
        let sp = p + sun_dir * t_light;
        acc = acc + cloud_density(sp, time_sec) * light_step_size;
    }
    // Beer-Lambert transmittance through the accumulated density.
    return exp(-acc * 0.8);
}

// ---- Main fragment ---------------------------------------------------------

struct FsOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fs_main(in: VertexOutput) -> FsOut {
    let dir = view_direction(in.uv);

    // Vertical gradient: 0 = horizon-down, 1 = zenith.
    let t = clamp(dir.y * 0.5 + 0.5, 0.0, 1.0);
    var color = mix(sky.bottom_color.rgb, sky.top_color.rgb, t);

    // Skip clouds entirely when disabled or when looking down.
    if (sky.cloud_params.x > 0.001 && dir.y > 0.0) {
        let origin = camera.position;
        let slab = ray_slab(origin, dir);
        if (slab.hit) {
            let t_start = max(slab.t_enter, 0.0);
            // Cap march length: 500 world units caps the cost of grazing
            // rays (which would otherwise march through the whole slab at
            // shallow angles) without visibly clipping cumulus at normal
            // viewing ranges. Was 800 in MVP; dropping to 500 saves ~35%
            // on horizon-direction pixels.
            let t_end   = min(slab.t_exit, t_start + 500.0);
            // 32 primary steps + hash jitter is the sweet spot for cumulus
            // at these bounds; going below 24 starts to show banding even
            // with jitter. Was 48; 32 saves ~33% of the primary march cost.
            let step_count = 32;
            let step_size = (t_end - t_start) / f32(step_count);

            // Jitter start to break banding.
            let jitter = hash13(vec3<f32>(in.position.xy, sky.wind_time.w));
            var t_march = t_start + step_size * jitter;

            let sun_dir = normalize(sky.sun_direction.xyz);
            let cos_theta = dot(dir, sun_dir);
            let phase = phase_hg(cos_theta, 0.5);

            var scattered = vec3<f32>(0.0);
            var transmittance = 1.0;
            let time_sec = sky.wind_time.w;

            for (var i = 0; i < step_count; i = i + 1) {
                // Early-out: below 5% transmittance the remaining samples
                // contribute <5% to final color; invisible to the eye.
                // Was 1%; at 5% we skip an extra 1-2 samples in dense
                // clouds with no visible loss.
                if (transmittance < 0.05) {
                    break;
                }
                let sample_pos = origin + dir * t_march;
                let density = cloud_density(sample_pos, time_sec);
                // Skip light march + composition for near-zero density.
                // The 0.02 threshold is slightly below the perceptible
                // smear seen at default coverage; raising it saves the
                // expensive `light_transmittance` call on cloud edges.
                if (density > 0.02) {
                    let extinction = density * step_size;
                    let sample_transmit = exp(-extinction * 1.5);

                    // In-scattering from the sun → single scattering approx.
                    let light_t = light_transmittance(sample_pos, sun_dir, time_sec);
                    let ambient = sky.top_color.rgb * 0.25; // mild ambient fill
                    let in_scatter = sky.sun_color.rgb * light_t * phase * 4.0 + ambient;

                    // Integral form: transmitted energy × (1 - local transmittance).
                    let step_contrib = in_scatter * (1.0 - sample_transmit);
                    scattered = scattered + step_contrib * transmittance;
                    transmittance = transmittance * sample_transmit;
                }
                t_march = t_march + step_size;
            }

            // Compose clouds over the sky gradient: the residual
            // transmittance lets the sky show through, `scattered` adds the
            // energy reaching the camera from in-scattered sunlight.
            color = color * transmittance + scattered;
        }
    }

    // Sun disk — a small highlight along the sun direction. Drawn after
    // clouds so it's occluded by opaque cloud regions naturally through
    // the transmittance multiplier above.
    let sun_dir = normalize(sky.sun_direction.xyz);
    let cos_sun = dot(dir, sun_dir);
    let sun_glow = pow(max(cos_sun, 0.0), 256.0);
    color = color + sky.sun_color.rgb * sun_glow * 4.0;

    var out: FsOut;
    out.color = vec4<f32>(color, 1.0);
    out.depth = 1.0;
    return out;
}
