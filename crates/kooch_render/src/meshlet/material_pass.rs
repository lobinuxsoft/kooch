//! Two-pass material shading for the meshlet R64 path (#440).

/// Fullscreen fragment shader that resolves `material_id` into a depth
/// target. Entry points: `vs_fullscreen`, `fs_resolve_material_depth`.
pub const RESOLVE_MATERIAL_DEPTH_SHADER: &str =
    include_str!("../../shaders/resolve_material_depth.wgsl");

/// Geometry bindings + barycentric attribute reconstruction, shared by
/// BOTH shading paths (the R64 two-pass fragment route and the R32
/// compute deferred). Prepended in Rust; WGSL has no `#include`.
pub const SURFACE_RECONSTRUCT_SHADER: &str = include_str!("../../shaders/surface_reconstruct.wgsl");

/// The R64 path's visibility-buffer read: the 64-bit storage binding, the frame uniforms, and
/// `resolve_vertex_output`. Pairs with [`SURFACE_RECONSTRUCT_SHADER`], which owns everything
/// downstream of the read — see [`compose_material_shader`].
pub const VISIBILITY_BUFFER_RESOLVE_SHADER: &str =
    include_str!("../../shaders/visibility_buffer_resolve.wgsl");

/// The fragment frame a surface shader runs inside. Entry points: `vs_fullscreen`, `fs_material`.
pub const MATERIAL_FRAGMENT_FRAME: &str =
    include_str!("../../shaders/material_frame_fragment.wgsl");

/// The compute frame: owns a 16x16 screen tile and reads that tile's froxel light list into
/// workgroup memory once (#824). Entry point: `cs_shade_tile`.
pub const MATERIAL_COMPUTE_FRAME: &str = include_str!("../../shaders/material_frame_compute.wgsl");

/// The contract between frames and surface shaders: material bindings, `SurfaceInput` and
/// `SurfaceOutput` (#1157).
pub const MATERIAL_SURFACE_PRELUDE: &str = include_str!("../../shaders/material_surface.wgsl");

/// The engine's PBR surface, used by every material without a shader of its own.
pub const DEFAULT_SURFACE_SHADER: &str =
    include_str!("../../shaders/material_surface_default.wgsl");

/// Tile edge, in pixels, of [`MATERIAL_COMPUTE_FRAME`]'s workgroup.
/// Must match the `TILE_SIZE` the shader declares; the dispatch size is
/// derived from it.
pub const SHADING_TILE_SIZE: u32 = 16;

/// Bind group Inti's frame UBO + light storage occupy on this path. Groups 0..4 are the
/// vbuf/camera/screen, the meshlet pool, the material storage, the scene buffers and the
/// per-material textures — 5 is the first free index.
pub const MATERIAL_PASS_INTI_GROUP: u32 = 5;

/// Group-0 bindings the contact-shadow march takes on this path (#735).
/// The vbuf, camera and screen uniforms hold 0/1/2; these are the next
/// free, and they sit in group 0 because the depth buffer is per view.
pub const MATERIAL_PASS_CONTACT_UBO_BINDING: u32 = 3;
pub const MATERIAL_PASS_CONTACT_DEPTH_BINDING: u32 = 4;

/// Composes a complete material shader: the visibility-buffer resolve helpers, the contact-shadow
/// march, the Inti shading model, the debug views (or the stub that removes them), the surface
/// contract, the surface body, then the frame. Stands in for a WGSL `#import`.
pub fn compose_material_shader(frame: &str, surface: &str, debug: bool) -> String {
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
        MATERIAL_SURFACE_PRELUDE,
        surface,
        frame,
    ]
    .join("\n")
}

/// Checks a surface shader against both frames before any pipeline is built from it, so a broken
/// edit is a message rather than a wgpu validation panic. Line numbers are the surface file's own.
pub fn validate_surface(surface: &str) -> Result<(), String> {
    // Everything composed ahead of the surface, so a message points into the file the author has.
    let before = compose_material_shader("", "", false).lines().count() - 1;
    let at = |location: Option<naga::SourceLocation>| {
        location
            .map(|l| format!("line {}: ", (l.line_number as usize).saturating_sub(before)))
            .unwrap_or_default()
    };
    // A binding or entry point the layout lacks would fail inside wgpu instead.
    let shape = |module: &naga::Module| {
        let bound = module.global_variables.iter();
        let bound = bound.filter(|(_, g)| g.binding.is_some()).count();
        (bound, module.entry_points.len())
    };
    for frame in [MATERIAL_FRAGMENT_FRAME, MATERIAL_COMPUTE_FRAME] {
        let composed = compose_material_shader(frame, surface, false);
        let module = naga::front::wgsl::parse_str(&composed)
            .map_err(|e| format!("{}{}", at(e.location(&composed)), e.message()))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|e| {
            let location = e.spans().next().map(|(span, _)| span.location(&composed));
            format!("{}{}", at(location), e.as_inner())
        })?;
        let reference = compose_material_shader(frame, DEFAULT_SURFACE_SHADER, false);
        let reference =
            naga::front::wgsl::parse_str(&reference).map_err(|e| e.message().to_owned())?;
        if shape(&module) != shape(&reference) {
            return Err(
                "a surface shader declares no bindings or entry points of its own — \
                        use the material's textures and `materials[input.material_id]`"
                    .to_owned(),
            );
        }
    }
    Ok(())
}

/// Depth format the material-depth target uses. 16-bit unorm gives an
/// exact `id / 65535` round-trip for up to 65 536 materials and lets the
/// per-material passes use a cheap hardware `Equal` depth test.
pub const MATERIAL_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

#[cfg(test)]
mod tests;
