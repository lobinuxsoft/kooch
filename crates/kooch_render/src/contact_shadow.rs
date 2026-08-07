//! Screen-space contact shadows (#735) — the Rust half.
//!
//! The shader lives in `shaders/contact_shadow.wgsl` and explains the
//! technique; what is here is the uniform it reads, the settings an
//! author edits, and the two bindings each shading path has to provide.
//!
//! # Why both shading paths get this
//!
//! The R64 two-pass fragment route and the R32 compute deferred shade in
//! different shaders, and nothing but this module stands between them
//! diverging. `inti_shade` calls `inti_contact_shadow` unconditionally,
//! so a path that does not concatenate this chunk fails to compile
//! rather than quietly rendering without contact shadows on the hardware
//! nobody develops on.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// The bindings, the four view helpers Bevy's march imports, and the
/// noise. Substitute the bindings with [`contact_shadow_shader`].
const CONTACT_SHADOW_PRELUDE: &str = include_str!("../shaders/contact_shadow.wgsl");

/// **Bevy 0.19's `bevy_pbr::raymarch`, ported literally.** Diff it
/// against upstream rather than reasoning about it — see its header for
/// why it is a copy and not a rewrite.
const BEVY_RAYMARCH: &str = include_str!("../shaders/bevy_raymarch.wgsl");

/// Bevy's `calculate_contact_shadow`, plus the probe a debug view reads
/// and the name the shading model calls.
const CONTACT_SHADOW_APPLY: &str = include_str!("../shaders/contact_shadow_apply.wgsl");

const UBO_PLACEHOLDER: &str = "{{CONTACT_SHADOW_UBO_BINDING}}";
const DEPTH_PLACEHOLDER: &str = "{{CONTACT_SHADOW_DEPTH_BINDING}}";

/// The march bound at the caller's own free bindings **in group 0**.
///
/// Group 0 and not a group of its own for two reasons that agree: the
/// bind-group budget is fully spent (six groups, six used), and the
/// depth buffer is a **per-view** resource, so it belongs beside the
/// other per-view bindings rather than in Inti's group, which is shared
/// across views. A per-view resource in that group is what made shadows
/// disappear the moment the light buffer grew, and the technique that
/// needs the depth buffer is not the place to repeat it.
pub fn contact_shadow_shader(ubo_binding: u32, depth_binding: u32) -> String {
    [
        CONTACT_SHADOW_PRELUDE
            .replace(UBO_PLACEHOLDER, &ubo_binding.to_string())
            .replace(DEPTH_PLACEHOLDER, &depth_binding.to_string()),
        BEVY_RAYMARCH.to_string(),
        CONTACT_SHADOW_APPLY.to_string(),
    ]
    .join("\n")
}

/// What the author decided contact shadows look like.
///
/// Global rather than per light: the length of a contact shadow is a
/// property of the scene's scale, not of which lamp is on. The per-light
/// switch is [`GpuLight::FLAG_CONTACT_SHADOWS`](kooch_lighting::GpuLight::FLAG_CONTACT_SHADOWS),
/// which decides *whether* a light marches, not how far.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactShadowSettings {
    /// Steps along the ray. **Zero turns the feature off** everywhere,
    /// whatever the lights say.
    pub linear_steps: u32,
    /// Assumed thickness of a depth-buffer fragment, in METRES. The
    /// buffer records a surface and the march needs a solid.
    pub thickness: f32,
    /// Ray length in METRES — how far from a surface an occluder can be
    /// and still ground it.
    pub length: f32,
}

impl Default for ContactShadowSettings {
    /// Bevy 0.19's values, unchanged. They are tuned against a metre-scale
    /// scene, which is the scale this engine's default scene is authored
    /// at; a project on a different scale will want `length` in
    /// proportion, which is why it is an author setting and not a
    /// constant.
    fn default() -> Self {
        Self {
            linear_steps: 16,
            thickness: 0.1,
            length: 0.3,
        }
    }
}

/// The march's per-view uniform. 96 bytes; mirrors `ContactShadowView`
/// in `contact_shadow.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ContactShadowUbo {
    pub view_proj: [[f32; 4]; 4],
    /// The camera's near plane. Under the engine's reversed-Z projection
    /// with no far plane, this alone linearises depth: `ndc.z` is
    /// `near / distance`. Bevy reads the same number out of
    /// `clip_from_view[3][2]` and calls it `perspective_camera_near()`.
    pub near: f32,
    pub length: f32,
    pub thickness: f32,
    pub linear_steps: u32,
    pub frame: u32,
    pub _pad: [u32; 3],
}

impl ContactShadowUbo {
    /// One view's uniform for this frame.
    ///
    /// `frame` only drives the jitter, so it may be any counter that
    /// advances; it wraps in the shader.
    pub fn new(view_proj: Mat4, near: f32, settings: &ContactShadowSettings, frame: u32) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            near,
            length: settings.length,
            thickness: settings.thickness,
            linear_steps: settings.linear_steps,
            frame,
            _pad: [0; 3],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    /// The march recovers metres as `near / ndc.z`, and that identity is
    /// the entire reason the camera lost its far plane. Checked against
    /// the projection the camera actually builds, not against the
    /// algebra that motivated it.
    #[test]
    fn near_over_ndc_z_is_the_distance_under_the_engines_projection() {
        let near = 0.1;
        let proj = crate::projection::perspective_infinite_rh_reverse_z(
            60.0_f32.to_radians(),
            16.0 / 9.0,
            near,
        );
        for distance in [0.1_f32, 0.5, 2.0, 37.0, 500.0, 100_000.0] {
            let clip = proj * glam::Vec4::new(0.0, 0.0, -distance, 1.0);
            let recovered = near / (clip.z / clip.w);
            assert!(
                (recovered - distance).abs() < distance * 1e-3,
                "at {distance} m the march would think it was at {recovered} m",
            );
        }
    }

    #[test]
    fn the_uniform_matches_the_shader_struct() {
        // ContactShadowView: mat4x4 (64) + near/length/thickness
        // (64..76) + linear_steps (76..80) + frame (80..84) + three
        // scalar pad words (84..96). Scalars rather than a vec2 on
        // purpose — see the shader.
        assert_eq!(std::mem::size_of::<ContactShadowUbo>(), 96);
    }

    #[test]
    fn substitution_leaves_no_placeholder_behind() {
        let src = contact_shadow_shader(3, 4);
        assert!(
            !src.contains("{{"),
            "a surviving placeholder fails to parse"
        );
        assert!(src.contains("@group(0) @binding(3)"));
        assert!(src.contains("@group(0) @binding(4)"));
    }

    /// Zero steps is the off switch, and it has to be reachable from the
    /// settings rather than only from a light's flag — a project that
    /// wants none of this should not pay a uniform read per light.
    #[test]
    fn zero_steps_is_expressible() {
        let settings = ContactShadowSettings {
            linear_steps: 0,
            ..Default::default()
        };
        let ubo = ContactShadowUbo::new(Mat4::IDENTITY, 0.1, &settings, 0);
        assert_eq!(ubo.linear_steps, 0);
    }
}
