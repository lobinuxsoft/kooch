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
    /// March once per pixel, for the light that lit it hardest, instead
    /// of once for every light that reaches it (#845).
    ///
    /// 🔴 The march is linear in taps and had no cap: measured on the
    /// OneXFly it costs 1.7 ms per step, and ~14 lights reach a pixel in
    /// a lit scene — the whole 13.9 ms frame budget, spent on contact.
    /// Every one of those marches interrogates the same depth buffer
    /// about the same point and differs only in direction.
    ///
    /// What it costs is the contact of the second-brightest lamp. In a
    /// scene lit by fourteen that was already diluted past seeing, by
    /// the same arithmetic that makes one light's shadow invisible among
    /// many. Turn it off for a scene lit by two or three, where each
    /// contact carries.
    pub dominant_only: bool,
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
            dominant_only: true,
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
    pub dominant_only: u32,
    pub _pad: [u32; 2],
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
            dominant_only: u32::from(settings.dominant_only),
            _pad: [0; 2],
        }
    }
}

/// `KOOCH_CONTACT_SHADOW_STEPS=<count>`, read once. `0` marches nothing.
///
/// 🔴 The variable exists because **the editor is not where this can be
/// measured**: the frame this answers for is a game on the OneXFly
/// launched through Steam, and `KOOCH_CLUSTERING`, `KOOCH_SPECULAR_FLOOR`,
/// `KOOCH_COMPUTE_SHADING` and `KOOCH_SHADING_RATE` all learned that the
/// same way. The asset field already exists (#830) and reaching it means
/// repacking and copying a build to the device, which changes two things
/// at once.
///
/// The count and not an on/off switch: the march is the one term in
/// `shade: compute` with no cap of any kind, so `16 → 8 → 4 → 0` says
/// whether the cost is the taps or the setup, and a switch only says
/// whether the whole thing is free.
///
/// Anything unparseable is `None`, the same as unset — a typo during a
/// measurement run must not silently change what is being measured, nor
/// override the author's value.
pub(crate) fn steps_from_environment() -> Option<u32> {
    static STEPS: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *STEPS.get_or_init(|| {
        let steps = parse_steps(std::env::var("KOOCH_CONTACT_SHADOW_STEPS").ok().as_deref());
        match steps {
            Some(0) => tracing::info!(
                target: "kooch_render::contact_shadow",
                "KOOCH_CONTACT_SHADOW_STEPS=0: no light marches the depth buffer, \
                 whatever the lights and the settings asset say",
            ),
            Some(count) => tracing::info!(
                target: "kooch_render::contact_shadow",
                "KOOCH_CONTACT_SHADOW_STEPS={count}: each light that reaches a pixel \
                 takes {count} depth taps instead of the asset's value",
            ),
            None => {}
        }
        steps
    })
}

/// The parse, apart from the read, so a test can exercise it without
/// touching the process environment.
fn parse_steps(raw: Option<&str>) -> Option<u32> {
    raw?.trim().parse().ok()
}

/// `KOOCH_CONTACT_SHADOW_DOMINANT=on` (or `off`), read once (#845).
///
/// Same reason as every other variable in this family: the A/B that
/// decides this runs on the OneXFly through Steam, and reaching the
/// settings asset there costs a repack and a copy — two changes where
/// the measurement needs one.
pub(crate) fn dominant_from_environment() -> Option<bool> {
    static DOMINANT: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *DOMINANT.get_or_init(|| {
        let dominant = parse_dominant(
            std::env::var("KOOCH_CONTACT_SHADOW_DOMINANT")
                .ok()
                .as_deref(),
        );
        if let Some(value) = dominant {
            tracing::info!(
                target: "kooch_render::contact_shadow",
                "KOOCH_CONTACT_SHADOW_DOMINANT={}: the march runs {}",
                if value { "on" } else { "off" },
                if value {
                    "once per pixel, for the light that lit it hardest"
                } else {
                    "once for every light that reaches the pixel"
                },
            );
        }
        dominant
    })
}

fn parse_dominant(raw: Option<&str>) -> Option<bool> {
    match raw.map(str::trim) {
        Some("on") | Some("1") => Some(true),
        Some("off") | Some("0") => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
