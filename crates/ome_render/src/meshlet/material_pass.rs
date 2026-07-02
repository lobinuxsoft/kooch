//! Two-pass material shading for the meshlet R64 path (#440).
//!
//! Replaces the single compute `cs_shade_scene_r64` with Bevy's / UE5
//! Nanite's two-pass model:
//!
//! 1. **Material-depth resolve** ([`RESOLVE_MATERIAL_DEPTH_SHADER`]): a
//!    fullscreen fragment pass reads the visibility buffer and writes
//!    each pixel's `material_id` into a [`MATERIAL_DEPTH_FORMAT`] target,
//!    encoded as `f32(id) / 65535.0`.
//! 2. **Per-material shading** (lands with the default PBR material
//!    shader): one fragment pass per registered material, each binding
//!    its own textures and depth-testing `Equal` against the material
//!    depth so it only shades its own pixels — the depth test is the
//!    material cull, for free.
//!
//! WGSL has no `#include`, so shared reconstruction helpers are
//! concatenated in Rust before module creation rather than imported.

/// Fullscreen fragment shader that resolves `material_id` into a depth
/// target. Entry points: `vs_fullscreen`, `fs_resolve_material_depth`.
pub const RESOLVE_MATERIAL_DEPTH_SHADER: &str =
    include_str!("../../shaders/resolve_material_depth.wgsl");

/// Depth format the material-depth target uses. 16-bit unorm gives an
/// exact `id / 65535` round-trip for up to 65 536 materials and lets the
/// per-material passes use a cheap hardware `Equal` depth test.
pub const MATERIAL_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_material_depth_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(RESOLVE_MATERIAL_DEPTH_SHADER)
            .expect("resolve_material_depth.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("resolve_material_depth.wgsl should validate");
    }
}
