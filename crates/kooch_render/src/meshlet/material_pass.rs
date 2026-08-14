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

/// Geometry bindings + barycentric attribute reconstruction, shared by
/// BOTH shading paths (the R64 two-pass fragment route and the R32
/// compute deferred). Prepended in Rust; WGSL has no `#include`.
pub const SURFACE_RECONSTRUCT_SHADER: &str = include_str!("../../shaders/surface_reconstruct.wgsl");

/// The R64 path's visibility-buffer read: the 64-bit storage binding,
/// the frame uniforms, and `resolve_vertex_output`. Pairs with
/// [`SURFACE_RECONSTRUCT_SHADER`], which owns everything downstream of
/// the read — see [`compose_material_shader`].
pub const VISIBILITY_BUFFER_RESOLVE_SHADER: &str =
    include_str!("../../shaders/visibility_buffer_resolve.wgsl");

/// Default per-material shading body: sampled albedo + tangent-space
/// normal mapping + metal/roughness, shaded by Inti. Compose it with
/// [`compose_material_shader`] before creating the module. Entry
/// points: `vs_fullscreen`, `fs_material`.
pub const MATERIAL_PBR_DEFAULT_BODY: &str = include_str!("../../shaders/material_pbr_default.wgsl");

/// The same shading as [`MATERIAL_PBR_DEFAULT_BODY`], as a compute entry
/// point that owns a 16x16 screen tile and reads that tile's froxel
/// light list into workgroup memory once (#824). Compose it with
/// [`compose_material_shader`] — it takes the identical prefix, which is
/// what keeps the two paths' arithmetic the same. Entry point:
/// `cs_shade_tile`.
pub const MATERIAL_PBR_COMPUTE_BODY: &str = include_str!("../../shaders/material_pbr_compute.wgsl");

/// Tile edge, in pixels, of [`MATERIAL_PBR_COMPUTE_BODY`]'s workgroup.
/// Must match the `TILE_SIZE` the shader declares; the dispatch size is
/// derived from it.
pub const SHADING_TILE_SIZE: u32 = 16;

/// Bind group Inti's frame UBO + light storage occupy on this path.
/// Groups 0..4 are the vbuf/camera/screen, the meshlet pool, the
/// material storage, the scene buffers and the per-material textures —
/// 5 is the first free index.
pub const MATERIAL_PASS_INTI_GROUP: u32 = 5;

/// Group-0 bindings the contact-shadow march takes on this path (#735).
/// The vbuf, camera and screen uniforms hold 0/1/2; these are the next
/// free, and they sit in group 0 because the depth buffer is per view.
pub const MATERIAL_PASS_CONTACT_UBO_BINDING: u32 = 3;
pub const MATERIAL_PASS_CONTACT_DEPTH_BINDING: u32 = 4;

/// Composes a complete material shader: the visibility-buffer resolve
/// helpers, the contact-shadow march, then the Inti shading model, then
/// the debug views (or the stub that removes them), then the
/// material-specific body. This stands in for the `#import` a WGSL
/// preprocessor would provide.
///
/// Order matters — WGSL resolves top to bottom, so anything the body
/// calls has to be declared above it. The march goes ahead of the
/// shading model for exactly that reason: `inti_shade` calls it. The
/// debug views go after it, because they call *it*.
///
/// `debug` builds the editor's variant. With it false the result
/// contains no debug view at all — see [`kooch_lighting::INTI_DEBUG_STUB`]
/// for why that is a performance decision and not tidiness (#743).
pub fn compose_material_shader(material_body: &str, debug: bool) -> String {
    let contact = crate::contact_shadow::contact_shadow_shader(
        MATERIAL_PASS_CONTACT_UBO_BINDING,
        MATERIAL_PASS_CONTACT_DEPTH_BINDING,
    );
    let inti = kooch_lighting::inti_pbr_shader(MATERIAL_PASS_INTI_GROUP);
    let debug_views = if debug {
        kooch_lighting::inti_debug_shader()
    } else {
        kooch_lighting::INTI_DEBUG_STUB
    };
    [
        VISIBILITY_BUFFER_RESOLVE_SHADER,
        SURFACE_RECONSTRUCT_SHADER,
        &contact,
        &inti,
        debug_views,
        material_body,
    ]
    .join("\n")
}

/// Depth format the material-depth target uses. 16-bit unorm gives an
/// exact `id / 65535` round-trip for up to 65 536 materials and lets the
/// per-material passes use a cheap hardware `Equal` depth test.
pub const MATERIAL_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

#[cfg(test)]
mod tests;
