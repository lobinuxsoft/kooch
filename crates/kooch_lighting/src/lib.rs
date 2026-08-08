//! **Inti** — the lighting system.
//!
//! Named for the Inca sun. The name covers the whole thing, not this
//! crate alone: extraction, the GPU light record, the shading model,
//! shadows when they land, clustering, light textures, global
//! illumination. When something says "the Inti path" it means the same
//! way "the meshlet path" does.
//!
//! # What this crate owns
//!
//! - [`GpuLight`] — one light as the shader reads it, and the
//!   conversion from a light component plus its transform.
//! - [`extract_lights`] — the ECS walk, pure and GPU-free.
//! - [`GpuLights`] — buffer residency and the bind group.
//! - [`AmbientLight`] / [`Exposure`] / [`PhysicalCamera`] — the
//!   per-frame constants, and the camera settings that make exposure a
//!   control a person can use.
//! - [`inti_pbr_shader`] — the WGSL every shading path concatenates.
//!
//! The shading model lives in `shaders/inti_pbr.wgsl` next to the Rust
//! struct it mirrors, deliberately: the two layouts have no compiler
//! between them, and putting them in separate crates is how they drift.
//!
//! # What it does not own
//!
//! Binding the group and calling `inti_shade` is each render path's
//! job — `kooch_render` depends on this crate, never the other way
//! round. There are two shading paths today (R64 two-pass fragment,
//! R32 compute deferred) and the maths must not be duplicated per
//! path, which is what [`inti_pbr_shader`] exists to prevent.

mod buffer;
mod extract;
mod frame;
mod gpu_light;

pub use buffer::GpuLights;
pub use extract::{extract_lights, shadow_casting_sun};
pub use frame::{
    AmbientLight, DEFAULT_SUN_SOFTNESS, Exposure, FRAME_CASCADE_COUNT, FrameShadows, GpuCascade,
    IntiFrame, PhysicalCamera,
};
pub use gpu_light::{
    GpuLight, LIGHT_KIND_DIRECTIONAL, LIGHT_KIND_POINT, LIGHT_KIND_SPOT, spot_cone_mad,
};

/// The shading model, as a template. Use [`inti_pbr_shader`].
const INTI_PBR_TEMPLATE: &str = include_str!("../shaders/inti_pbr.wgsl");

/// Placeholder the template carries where the bind-group index goes.
const GROUP_PLACEHOLDER: &str = "{{INTI_GROUP}}";

/// The shading model as WGSL, bound at `group`.
///
/// WGSL has no `#include` and no way to parameterise `@group`, so the
/// index is substituted textually and the result is concatenated ahead
/// of the consumer's own source — the same mechanism
/// `compose_material_shader` already uses for the visibility-buffer
/// resolve helpers.
///
/// Each path passes its own first free group: the R64 two-pass path has
/// 0..4 taken, the R32 compute path 0..3.
pub fn inti_pbr_shader(group: u32) -> String {
    INTI_PBR_TEMPLATE.replace(GROUP_PLACEHOLDER, &group.to_string())
}

/// What `inti_shade` calls for contact shadows (#735), when the composer
/// has nothing to march.
///
/// The march itself needs the scene depth buffer, which is a **per-view**
/// resource and so belongs in the consumer's own group rather than in
/// Inti's — that group is shared across views, and a per-view resource
/// living in it is what made shadows vanish once the light buffer grew.
/// So the shading model states the contract and `kooch_render` supplies
/// the implementation; this stub is what a path with no depth to sample
/// concatenates instead.
///
/// Naming a function the model calls but does not define is not a
/// weakness of the arrangement, it is the point: a path that forgets to
/// supply one of the two fails to compile rather than quietly shading
/// differently from its sibling.
pub const INTI_CONTACT_SHADOW_STUB: &str = "\
// No depth buffer to march — every light reports fully unoccluded, and
// the debug view says so rather than colouring a march that never ran.
struct ContactShadowProbe {
    shadow: f32,
    hit: bool,
    hit_t: f32,
    steps: u32,
    ray_px: f32,
}

fn inti_contact_shadow(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_camera: vec3<f32>,
    to_light: vec3<f32>,
    frag_coord: vec2<f32>,
) -> f32 {
    return 1.0;
}

fn inti_contact_shadow_probe(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_camera: vec3<f32>,
    to_light: vec3<f32>,
    frag_coord: vec2<f32>,
) -> ContactShadowProbe {
    return ContactShadowProbe(1.0, false, 0.0, 0u, 0.0);
}

fn inti_contact_shadow_debug(probe: ContactShadowProbe) -> vec3<f32> {
    return vec3<f32>(0.0);
}
";

#[cfg(test)]
mod tests;
