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

/// Attribute-reconstruction helpers (barycentrics, analytical uv
/// derivatives, `resolve_vertex_output`) shared by the per-material
/// shading passes. WGSL has no `#include`, so this is prepended in Rust
/// to each material shader — see [`compose_material_shader`].
pub const VISIBILITY_BUFFER_RESOLVE_SHADER: &str =
    include_str!("../../shaders/visibility_buffer_resolve.wgsl");

/// Default per-material shading body (normal-debug × albedo, tangent-space
/// normal mapping). Concatenate with [`VISIBILITY_BUFFER_RESOLVE_SHADER`]
/// via [`compose_material_shader`] before creating the module. Entry
/// points: `vs_fullscreen`, `fs_material`.
pub const MATERIAL_PBR_DEFAULT_BODY: &str = include_str!("../../shaders/material_pbr_default.wgsl");

/// Composes a complete material shader by prepending the shared
/// visibility-buffer resolve helpers to a material-specific body. This
/// stands in for the `#import` a WGSL preprocessor would provide.
pub fn compose_material_shader(material_body: &str) -> String {
    format!("{VISIBILITY_BUFFER_RESOLVE_SHADER}\n{material_body}")
}

/// Depth format the material-depth target uses. 16-bit unorm gives an
/// exact `id / 65535` round-trip for up to 65 536 materials and lets the
/// per-material passes use a cheap hardware `Equal` depth test.
pub const MATERIAL_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(source: &str, what: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{what} should parse: {e}"));
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .unwrap_or_else(|e| panic!("{what} should validate: {e}"));
    }

    #[test]
    fn resolve_material_depth_parses_and_validates() {
        validate(RESOLVE_MATERIAL_DEPTH_SHADER, "resolve_material_depth.wgsl");
    }

    #[test]
    fn visibility_buffer_resolve_parses_and_validates() {
        validate(
            VISIBILITY_BUFFER_RESOLVE_SHADER,
            "visibility_buffer_resolve.wgsl",
        );
    }

    #[test]
    fn composed_default_material_parses_and_validates() {
        let composed = compose_material_shader(MATERIAL_PBR_DEFAULT_BODY);
        validate(&composed, "composed default material shader");
    }
}
