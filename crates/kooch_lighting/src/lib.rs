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
mod cluster;
mod extract;
mod frame;
mod gpu_light;

pub use buffer::GpuLights;
pub use cluster::{
    ClusterCamera, ClusterDraw, ClusterGrid, ClusterSettings, ClusterViewUniform, GpuClusters,
};
pub mod light_frame;
pub use extract::{
    ExtractedLights, PointShadowSource, SpotShadowSource, point_shadow_importance, shadow_note,
};
pub use frame::{
    AmbientLight, DEFAULT_SUN_SOFTNESS, DebugLight, Exposure, FRAME_CASCADE_COUNT, FrameShadows,
    GpuCascade, GpuPointShadow, IntiFrame, LIGHTS_HOT_DEFAULT, LightLimit, LightsHot,
    MAX_POINT_SHADOWS, MAX_SPOT_SHADOWS, NO_DEBUG_LIGHT, PhysicalCamera, SpecularFloor,
};
pub use gpu_light::{
    GpuLight, LIGHT_KIND_DIRECTIONAL, LIGHT_KIND_POINT, LIGHT_KIND_SPOT, NO_SHADOW_SLOT,
    spot_cone_mad,
};
pub use light_frame::LightFrame;

/// The shading model, as a template. Use [`inti_pbr_shader`].
const INTI_PBR_TEMPLATE: &str = include_str!("../shaders/inti_pbr.wgsl");

/// Placeholder the template carries where the bind-group index goes.
const GROUP_PLACEHOLDER: &str = "{{INTI_GROUP}}";

/// Placeholder for [`MAX_POINT_SHADOWS`], which sizes an array the Rust
/// side declares too.
///
/// 🔴 Substituted rather than written in both places. The number was
/// literal `4` in the WGSL and a constant in Rust, with nothing between
/// them: raising one and not the other reads every field after the array
/// at the wrong offset and **does not fail to compile**. The same shape
/// of defect as the seven files that declare `MeshInstance`.
const POINT_SHADOWS_PLACEHOLDER: &str = "{{INTI_MAX_POINT_SHADOWS}}";

/// The debug views. Concatenated only by a pipeline that can show them.
const INTI_DEBUG_SOURCE: &str = include_str!("../shaders/inti_debug.wgsl");

/// The tonemap operator, with no bindings of its own (#732).
///
/// [`inti_pbr_shader`] already prepends it, so a shading path gets it
/// for free. The standalone tonemap pass concatenates **this alone**:
/// it has an HDR texture and an exposure scalar, and pulling in the
/// whole shading model to reach one curve would drag the light storage
/// buffer and the shadow atlas into a pass that samples neither.
pub const INTI_TONEMAP: &str = include_str!("../shaders/inti_tonemap.wgsl");

/// The froxel grid's shared declarations, for a pass built elsewhere.
///
/// Concatenated ahead of a shader body, the way the grid's own passes
/// do it: `ClusterView`, `ClusterCell`, `ClusterLight` and the two
/// lookups a reader of the grid needs. A pass that redeclared them would
/// be free to drift, and the drift would show as fragments reading a
/// cell the grid never wrote.
pub const CLUSTER_COMMON: &str = include_str!("../shaders/cluster_common.wgsl");

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
    // The tonemap first: `inti_tonemap` calls into it, and WGSL wants a
    // function declared before it is used.
    let mut out = String::from(INTI_TONEMAP);
    out.push_str(
        &INTI_PBR_TEMPLATE
            .replace(GROUP_PLACEHOLDER, &group.to_string())
            .replace(POINT_SHADOWS_PLACEHOLDER, &MAX_POINT_SHADOWS.to_string()),
    );
    out
}

/// The debug views as WGSL, to concatenate **after**
/// [`inti_pbr_shader`]. Everything in it reads bindings and helpers that
/// file declares, so it has no group of its own to substitute.
///
/// Pair it with a pipeline the editor builds on demand. A shipped game
/// concatenates [`INTI_DEBUG_STUB`] instead and never compiles a line of
/// this.
pub fn inti_debug_shader() -> &'static str {
    INTI_DEBUG_SOURCE
}

/// What a production pipeline concatenates where the debug views would
/// go (#743).
///
/// # Why this is not just three untaken `if`s
///
/// A branch nothing takes is still code the shader carries. Register
/// allocation is worst-case across the whole entry point, so a cascade
/// sample and a screen-space raymarch parked in a dead branch still
/// raise the VGPR count — and VGPR count is what caps how many waves
/// stay in flight, which is the whole of an integrated GPU's ability to
/// hide memory latency. On the 10 W handheld budget this engine is held
/// to, that is not a rounding error.
///
/// So the game's shader does not contain them. `inti_debug_is_view`
/// returning a literal `false` is what removes the call sites: they fold
/// to `if (false)` before register allocation. The stub exists so both
/// pipelines compile against the same source, which is the only way the
/// two cannot drift.
pub const INTI_DEBUG_STUB: &str = "\
// No debug views in this pipeline. Both functions are dead weight the
// compiler folds away; they exist so the shading paths have one call
// site rather than a `#ifdef` this language does not have.
fn inti_debug_is_view(mode: u32) -> bool {
    return false;
}

fn inti_debug_view(
    mode: u32,
    world_position: vec3<f32>,
    n: vec3<f32>,
    frag_coord: vec2<f32>,
) -> vec3<f32> {
    return vec3<f32>(0.0);
}
";

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

// Nothing to march, so nothing to choose between: the shading model
// takes its per-light path and every call above returns unoccluded.
fn inti_contact_dominant_only() -> bool {
    return false;
}
";

#[cfg(test)]
mod tests;
