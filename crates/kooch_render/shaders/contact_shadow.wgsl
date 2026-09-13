// contact_shadow.wgsl — screen-space contact shadows (#735).

struct ContactShadowView {
    /// World → clip for this view. Bevy calls this `clip_from_world`.
    view_proj: mat4x4<f32>,
    /// The camera's near plane, in world units.
    near: f32,
    /// How far along the ray to light the march travels, in world units. Screen-space cost is
    /// independent of this: a longer ray is the same step count spread wider, which is why the
    /// technique costs the same on a crate and on a moon.
    ray_length: f32,
    /// The depth buffer is 2.5D — it records a surface, not a solid. This is the thickness every
    /// fragment is assumed to have, in world units. Too small and shadows detach; too large and
    /// thin geometry casts a shadow through everything behind it.
    thickness: f32,
    /// Steps along the ray. **Zero disables the whole feature** — the
    /// early-out below is the "off" switch, so a scene that never enables
    /// contact shadows pays one uniform read per light.
    linear_steps: u32,
    /// Frame counter, for the jitter. Without it the noise pattern is
    /// frozen into the image and reads as a texture rather than as noise.
    frame: u32,
    // 🔴 Three scalars, NOT `vec2<u32>`. A vec2 aligns to 8, which pushes it from offset 84 to 88
    // and leaves a hole `#[repr(C)]` does not have — the struct is then 96 bytes on one side and 92
    // on the other, and every field after the hole reads garbage.
    dominant_only: u32,
    /// Punctual lights per pixel that may march, or 0 for no cap.
    max_lights: u32,
    _pad0: u32,
}

@group(0) @binding({{CONTACT_SHADOW_UBO_BINDING}}) var<uniform> contact_shadow: ContactShadowView;
// The scene depth buffer, sampled rather than attached — during shading the material-depth target
// is what is bound, so this one is free to be read. Named for what Bevy names it, because the
// ported march reads it under that name and a rename is a diff against upstream.
@group(0) @binding({{CONTACT_SHADOW_DEPTH_BINDING}}) var depth_prepass_texture: texture_depth_2d;

// ── The four view helpers `bevy_pbr::raymarch` imports ──────────────

fn ndc_to_uv(ndc: vec2<f32>) -> vec2<f32> {
    return ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
}

fn position_world_to_ndc(world_pos: vec3<f32>) -> vec3<f32> {
    let ndc_pos = contact_shadow.view_proj * vec4<f32>(world_pos, 1.0);
    return ndc_pos.xyz / ndc_pos.w;
}

fn direction_world_to_clip(world_dir: vec3<f32>) -> vec4<f32> {
    return contact_shadow.view_proj * vec4<f32>(world_dir, 0.0);
}

fn perspective_camera_near() -> f32 {
    return contact_shadow.near;
}

// Interleaved gradient noise (Jimenez '14), from `bevy_pbr::utils`. Decorrelates the first step of
// neighbouring pixels so the march's quantization reads as dither rather than banding, and rotates
// per frame.
fn interleaved_gradient_noise(pixel_coordinates: vec2<f32>, frame: u32) -> f32 {
    let xy = pixel_coordinates + 5.588238 * f32(frame % 64u);
    return fract(52.9829189 * fract(0.06711056 * xy.x + 0.00583715 * xy.y));
}
