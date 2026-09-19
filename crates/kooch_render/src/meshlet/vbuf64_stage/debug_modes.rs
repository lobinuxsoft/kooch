//! What each debug view asks of the passes after the shade.

/// Reports the mismatched frame once, and never again.
pub(super) fn warn_once_about_transitional_frame(render: (u32, u32), output: (u32, u32)) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        tracing::warn!(
            target: "kooch_render::meshlet",
            render_size = ?render,
            output_size = ?output,
            "the fragment shading path was handed a reduced render size and skipped a              frame; the scale belongs to the compute path and the two settings landed              one frame apart",
        );
    });
}

/// True for every debug mode, which is the question the TONEMAP asks: all of them produce colour
/// that is already display-referred, and a false-colour legend through a filmic curve is a legend
/// nobody can read off.
pub(super) fn is_debug_view(debug_mode: u32) -> bool {
    debug_mode >= 11
}

/// True only for the modes Inti resolves INSIDE the shading shader, so there is no radiance for a
/// temporal technique to resolve.
pub(super) fn replaces_shading(debug_mode: u32) -> bool {
    crate::meshlet::debug::MeshletDebugMode::all_implemented()
        .iter()
        .find(|m| m.as_u32() == debug_mode)
        .is_some_and(|m| m.replaces_shading())
}

/// True when the tonemap must pass the colour through untouched.
pub(super) fn is_display_referred(debug_mode: u32) -> bool {
    crate::meshlet::debug::MeshletDebugMode::all_implemented()
        .iter()
        .find(|m| m.as_u32() == debug_mode)
        .is_some_and(|m| m.is_display_referred())
}

/// Which FSR 3.1 intermediate the debug dropdown is asking for, or 0.
pub(super) fn fsr3_debug_stage(debug_mode: u32) -> u32 {
    crate::meshlet::debug::MeshletDebugMode::all_implemented()
        .iter()
        .find(|m| m.as_u32() == debug_mode)
        .map_or(0, |m| m.fsr3_stage())
}
