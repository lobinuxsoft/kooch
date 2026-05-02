//! Tile-based ray bounds pre-pass.
//!
//! PR-6 of epic #370. A compute pre-pass walks the coarsest GDF
//! cascade once per 8×8 viewport tile, emitting a `(t_min, t_max,
//! flags)` triplet to a persistent SSBO. The fragment shader reads
//! the entry per pixel: `flags == 0` -> `discard` (sky pass + depth=1
//! survive), otherwise the ray-march loop is clamped to
//! `[t_min, t_max]`. Empty-sky tiles cost zero fragment work; non-
//! empty tiles ray-march a much smaller `t` interval than
//! `[0, max_distance]`.
//!
//! See [`crate::raymarch::RayMarchRenderer`] for the wiring that
//! creates a [`TileCullState`] alongside the GDF cascade resources,
//! dispatches it before the raymarch render pass, and exposes the
//! SSBO + UBO to the fragment shader on bind group 2.

mod state;
pub mod uniforms;

pub use state::TileCullState;
pub use uniforms::{TileBounds, TileCullUniforms, TILE_FLAG_NON_EMPTY, TILE_WORKGROUP_XY};

/// Compute-pre-pass shader source. Concatenated lazily — the host-side
/// `TileCullState::new` `include_str!`s the same path; this constant
/// is exposed so tests + tools can validate the shader without spinning
/// up the full state.
pub const TILE_CULL_SHADER_SOURCE: &str = include_str!("../../shaders/tile_cull.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    /// Naga must parse + validate `tile_cull.wgsl` standalone — the
    /// shader does NOT depend on the SDF / pool / GDF libraries, so a
    /// breaking change here surfaces at unit-test time rather than
    /// inside `TileCullState::new` on the GPU.
    #[test]
    fn tile_cull_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(TILE_CULL_SHADER_SOURCE)
            .expect("tile_cull.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("tile_cull.wgsl should validate");
        let entry_names: Vec<&str> = module
            .entry_points
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            entry_names.iter().any(|n| *n == "cs_tile_cull"),
            "tile_cull shader must expose `cs_tile_cull`; saw {entry_names:?}"
        );
    }
}
