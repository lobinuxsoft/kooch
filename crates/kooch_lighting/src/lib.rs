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
pub use extract::extract_lights;
pub use frame::{
    AmbientLight, DEFAULT_LIGHT_SIZE_WORLD, Exposure, FRAME_CASCADE_COUNT, GpuCascade, IntiFrame,
    PhysicalCamera,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitution_leaves_no_placeholder_behind() {
        let src = inti_pbr_shader(5);
        assert!(
            !src.contains(GROUP_PLACEHOLDER),
            "a surviving placeholder is a shader that fails to parse at \
             pipeline creation, which is a runtime panic and not a test failure",
        );
        assert!(src.contains("@group(5) @binding(0)"));
        assert!(src.contains("@group(5) @binding(1)"));
    }

    #[test]
    fn the_template_is_not_valid_wgsl_on_its_own() {
        // Guards the reverse mistake: someone including the template
        // directly instead of calling the function would get a parse
        // error at pipeline creation. Better to state the contract.
        assert!(INTI_PBR_TEMPLATE.contains(GROUP_PLACEHOLDER));
    }

    #[test]
    fn shading_model_parses_and_validates() {
        let module =
            naga::front::wgsl::parse_str(&inti_pbr_shader(0)).expect("inti_pbr.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("inti_pbr.wgsl should validate");
    }
}
