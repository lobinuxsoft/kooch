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

/// The frame a post-process shader runs inside: one full-screen draw over the colour the camera
/// produced (#1201). Entry points: `vs_fullscreen`, `fs_post`.
pub const MATERIAL_POST_FRAME: &str = include_str!("../../shaders/material_frame_post.wgsl");

/// The frame a transparent surface runs inside: its meshlets rasterised over the opaque scene and
/// blended (#452). Entry points: `vs_forward`, `fs_forward`.
pub const MATERIAL_FORWARD_FRAME: &str = include_str!("../../shaders/material_frame_forward.wgsl");

/// The Shader Graph preview's frame: one primitive, rasterised, lit by a key light of its own.
/// Entry points: `vs_preview`, `fs_preview`.
pub const MATERIAL_PREVIEW_FRAME: &str = include_str!("../../shaders/material_frame_preview.wgsl");

/// What New Shader writes: a PBR surface over its own declared parameters (#1158).
pub const NEW_SURFACE_SHADER: &str = include_str!("../../shaders/material_surface_template.wgsl");

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
/// contract, the code generated from the shader's parameters, the surface body, then the frame.
/// Stands in for a WGSL `#import`.
/// The GPU scope a material's shading is timed under: one label per shader, so every material using
/// it adds to the same number (#1159). `None` is the engine's built-in surface.
pub fn shader_scope(shader: Option<kooch_core::Guid>) -> String {
    match shader {
        Some(guid) => format!("shader {guid}"),
        None => "shader built-in".to_owned(),
    }
}

pub fn compose_material_shader(frame: &str, params: &str, surface: &str, debug: bool) -> String {
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
        params,
        surface,
        frame,
    ]
    .join("\n")
}

/// The preview's shader: the surface contract, the shader's own parameters, its body, and the
/// preview frame — **without** Inti, the contact-shadow march or the visibility-buffer resolve.
///
/// 🔴 A preview lights its own primitive, so none of that has to exist for it to run. The frame
/// declares the handful of things the contract reads (`screen`, `inti.camera_position`,
/// `VertexOutput`) and the surface never knows the difference.
pub fn compose_preview_shader(params: &str, surface: &str) -> String {
    [
        MATERIAL_SURFACE_PRELUDE,
        params,
        surface,
        MATERIAL_PREVIEW_FRAME,
    ]
    .join("\n")
}

/// A post-process shader's composition: the surface contract, the shader's parameters, its body and
/// the post frame — no Inti, no visibility buffer. The frame hands it the scene instead.
pub fn compose_post_shader(params: &str, post: &str) -> String {
    [MATERIAL_SURFACE_PRELUDE, params, post, MATERIAL_POST_FRAME].join("\n")
}

/// Checks a post-process shader before a pipeline is built from it. Line numbers are the file's own.
pub fn validate_post(params: &str, post: &str) -> Result<(), String> {
    // How many lines sit ahead of the body, measured rather than counted: a message has to point
    // into the file the author has, and the frame is composed after the body.
    const MARK: &str = "//__body__";
    let before = compose_post_shader(params, MARK)
        .lines()
        .position(|line| line == MARK)
        .unwrap_or(0);
    let composed = compose_post_shader(params, post);
    let at = |line: usize| format!("line {}: ", line.saturating_sub(before));
    let module = naga::front::wgsl::parse_str(&composed).map_err(|e| {
        let line = e.location(&composed).map(|l| l.line_number as usize);
        format!("{}{}", line.map(at).unwrap_or_default(), e.message())
    })?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map(|_| ())
    .map_err(|e| e.as_inner().to_string())
}

/// The same check for the preview's frame. A graph is edited node by node, and most of those
/// moments are a shader that does not compile yet — which has to read as a message in the panel,
/// never as a wgpu validation panic.
pub fn validate_preview(params: &str, surface: &str) -> Result<(), String> {
    let composed = compose_preview_shader(params, surface);
    let module = naga::front::wgsl::parse_str(&composed).map_err(|e| e.message().to_owned())?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map(|_| ())
    .map_err(|e| e.as_inner().to_string())
}

/// Checks a surface shader against both frames before any pipeline is built from it, so a broken
/// edit is a message rather than a wgpu validation panic. Line numbers are the surface file's own.
pub fn validate_surface(params: &str, surface: &str) -> Result<(), String> {
    // Everything composed ahead of the surface, so a message points into the file the author has.
    let before = compose_material_shader("", params, "", false)
        .lines()
        .count()
        - 1;
    let at = |location: Option<naga::SourceLocation>| {
        location
            .map(|l| format!("line {}: ", (l.line_number as usize).saturating_sub(before)))
            .unwrap_or_default()
    };
    // Bindings are checked when the shader is read; an entry point of its own would fail in wgpu.
    let reference = crate::material::Shader::default_surface();
    for frame in [
        MATERIAL_FRAGMENT_FRAME,
        MATERIAL_COMPUTE_FRAME,
        MATERIAL_FORWARD_FRAME,
    ] {
        let composed = compose_material_shader(frame, params, surface, false);
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
        let expected =
            compose_material_shader(frame, &reference.params_wgsl(), &reference.source, false);
        let expected =
            naga::front::wgsl::parse_str(&expected).map_err(|e| e.message().to_owned())?;
        if module.entry_points.len() != expected.entry_points.len() {
            return Err("a surface shader declares no entry points of its own".to_owned());
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
