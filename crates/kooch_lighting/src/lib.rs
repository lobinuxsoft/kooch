//! **Inti**, the lighting system — extraction, light records, shading, shadows, clustering. Owns
//! [`GpuLight`], [`LightFrame`], [`GpuLights`] and [`inti_pbr_shader`]; binding and `inti_shade`
//! calls are `kooch_render`'s.

mod buffer;
mod cluster;
mod extract;
mod frame;
mod gpu_light;

pub use buffer::{GpuLights, PageBinding};
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

/// Placeholder for [`MAX_POINT_SHADOWS`], substituted so WGSL and Rust cannot disagree — a mismatch
/// shifts every later field and still compiles.
const POINT_SHADOWS_PLACEHOLDER: &str = "{{INTI_MAX_POINT_SHADOWS}}";

/// The debug views. Concatenated only by a pipeline that can show them.
const INTI_DEBUG_SOURCE: &str = include_str!("../shaders/inti_debug.wgsl");

/// The tonemap operator alone (#732), for the standalone pass that has an HDR texture and exposure;
/// [`inti_pbr_shader`] already prepends it.
pub const INTI_TONEMAP: &str = include_str!("../shaders/inti_tonemap.wgsl");

/// The froxel grid's declarations for passes built elsewhere, so no redeclaration can drift.
pub const CLUSTER_COMMON: &str = include_str!("../shaders/cluster_common.wgsl");

/// The page table's declarations (#866), here because the reader is: four `kooch_render` passes
/// write the table and this crate's shading reads it, so they share one encoding.
pub const PAGE_TABLE: &str = include_str!("../shaders/page_table.wgsl");

/// The shading model as WGSL bound at `group`, substituted textually (WGSL has no `#include`). R64
/// two-pass takes groups 0..4, R32 compute 0..3.
pub fn inti_pbr_shader(group: u32) -> String {
    // The tonemap first: `inti_tonemap` calls into it, and WGSL wants a
    // function declared before it is used.
    let mut out = String::from(INTI_TONEMAP);
    // The page table's declarations, which `inti_page_shadow` reads. The
    // same file the four passes that FILL the table are built from.
    out.push_str(PAGE_TABLE);
    out.push('\n');
    out.push_str(
        &INTI_PBR_TEMPLATE
            .replace(GROUP_PLACEHOLDER, &group.to_string())
            .replace(POINT_SHADOWS_PLACEHOLDER, &MAX_POINT_SHADOWS.to_string()),
    );
    out
}

/// The debug views, concatenated after [`inti_pbr_shader`] in the editor's on-demand pipeline;
/// games use [`INTI_DEBUG_STUB`].
pub fn inti_debug_shader() -> &'static str {
    INTI_DEBUG_SOURCE
}

/// What production concatenates instead of the debug views (#743). An untaken branch still raises
/// VGPR count, which caps an integrated GPU at 10 W; `inti_debug_is_view` returning `false` folds
/// the call sites away.
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

#[cfg(test)]
/// `inti_contact_shadow` for paths with nothing to march. The march needs per-view depth, which
/// belongs in the consumer's group; `kooch_render` supplies it, and forgetting both fails to
/// compile.
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

fn inti_contact_max_lights() -> u32 {
    return 0u;
}
";

#[cfg(test)]
mod tests;
