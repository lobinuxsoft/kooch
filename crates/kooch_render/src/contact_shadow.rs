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

/// The march, as WGSL. Substitute the bindings with
/// [`contact_shadow_shader`].
const CONTACT_SHADOW_TEMPLATE: &str = include_str!("../shaders/contact_shadow.wgsl");

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
    CONTACT_SHADOW_TEMPLATE
        .replace(UBO_PLACEHOLDER, &ubo_binding.to_string())
        .replace(DEPTH_PLACEHOLDER, &depth_binding.to_string())
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
    /// `(x, y)` for `linear_z = x / (y - ndc_z)`. See [`depth_to_linear`].
    pub depth_to_linear: [f32; 2],
    pub length: f32,
    pub thickness: f32,
    pub linear_steps: u32,
    pub frame: u32,
    pub _pad: [u32; 2],
}

impl ContactShadowUbo {
    /// One view's uniform for this frame.
    ///
    /// `frame` only drives the jitter, so it may be any counter that
    /// advances; it wraps in the shader.
    pub fn new(
        view_proj: Mat4,
        near: f32,
        far: f32,
        settings: &ContactShadowSettings,
        frame: u32,
    ) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            depth_to_linear: depth_to_linear(near, far),
            length: settings.length,
            thickness: settings.thickness,
            linear_steps: settings.linear_steps,
            frame,
            _pad: [0; 2],
        }
    }
}

/// Coefficients that turn a stored reversed-Z depth back into metres:
/// `linear_z = x / (y - ndc_z)`.
///
/// 🔴 **Not Bevy's `1.0 / ndc_z`.** That holds only for a reversed-Z
/// projection with an infinite far plane, and
/// [`perspective_rh_reverse_z`](crate::perspective_rh_reverse_z) takes a
/// finite one — porting the identity along with the algorithm would have
/// scaled every depth in the march by a factor that varies with the far
/// plane, which reads as a thickness parameter that means something
/// different in every scene.
///
/// With `r = far / (near - far)`: the projection stores
/// `ndc_z = 1 - r * (near - d) / d`, and inverting gives `x = r * near`,
/// `y = 1 + r`. As `far` grows, `r → -1` and this becomes `near / ndc_z`,
/// which is Bevy's form — the check that it is the same maths generalised
/// and not different maths.
pub fn depth_to_linear(near: f32, far: f32) -> [f32; 2] {
    let r = far / (near - far);
    [r * near, 1.0 + r]
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    /// Round-trips real points through the real projection matrix rather
    /// than through the algebra that produced the coefficients: deriving
    /// an inverse and then testing it against itself proves nothing.
    #[test]
    fn the_coefficients_invert_the_engines_projection() {
        let (near, far) = (0.1, 1000.0);
        let proj = crate::perspective_rh_reverse_z(60.0_f32.to_radians(), 16.0 / 9.0, near, far);
        let [x, y] = depth_to_linear(near, far);

        for distance in [0.1, 0.5, 2.0, 37.0, 500.0, 1000.0] {
            // A point straight ahead at `distance`: the view axis is -Z.
            let clip = proj * Vec4::new(0.0, 0.0, -distance, 1.0);
            let ndc_z = clip.z / clip.w;
            let recovered = x / (y - ndc_z);
            assert!(
                (recovered - distance).abs() < distance * 1e-3,
                "at {distance} m the march would think it was at {recovered} m",
            );
        }
    }

    /// The far plane is what Bevy's shortcut assumes away, so pin that
    /// this generalisation collapses onto it rather than merely being
    /// near it.
    #[test]
    fn an_infinite_far_plane_collapses_to_bevys_form() {
        let near = 0.1;
        let [x, y] = depth_to_linear(near, 1.0e9);
        // Bevy: linear = near / ndc_z, i.e. x = -near and y = 0.
        assert!((x + near).abs() < 1e-6, "x was {x}");
        assert!(y.abs() < 1e-6, "y was {y}");
    }

    #[test]
    fn the_uniform_matches_the_shader_struct() {
        // ContactShadowView: mat4x4 (64) + vec2<f32> (8) + f32 + f32
        // (72..80) + u32 + u32 (80..88) + vec2<u32> (88..96).
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
        let ubo = ContactShadowUbo::new(Mat4::IDENTITY, 0.1, 100.0, &settings, 0);
        assert_eq!(ubo.linear_steps, 0);
    }
}
