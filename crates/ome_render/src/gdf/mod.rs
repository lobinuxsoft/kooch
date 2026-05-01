//! Global Distance Field (GDF) — compute-populated 3D cascade textures
//! that the raymarcher will later sample with a single fetch instead
//! of walking the TLAS+BLAS pool. Epic #370 phase 1.
//!
//! PR-3 ships **cascade 0 only** — 16 m cube, 64³ R16Float voxels,
//! voxel pitch 0.25 m. Per-frame compute pass evaluates
//! `eval_scene_bvh(world_pos)` at every voxel centre and writes the
//! signed distance into the cascade texture. **The fragment shader
//! does not yet consume the cascade** — that's PR-4. PR-3 exists
//! purely to validate that the cascade is being populated correctly.
//!
//! ## Bind-group split
//!
//! The populate compute pipeline uses TWO bind groups:
//! - **group 0** — cascade descriptor uniform + cascade storage 3D
//!   texture (this crate's bindings).
//! - **group 1** — pool buffers (`tlas_nodes`, `chunk_descriptors`,
//!   `bvh_nodes_pool`, `leaf_aabbs_pool`, `primitives_pool`,
//!   `tlas_uniforms`) at bindings 5..=10. Identical to the layout
//!   `raymarch_pool_eval.wgsl` declares. The library shader is
//!   concatenated byte-for-byte so re-binding under a different
//!   group/binding scheme would force a non-trivial textual rewrite.
//!
//! ## Shader concat order
//!
//! `sdf_primitives.wgsl` + `raymarch_pool_eval.wgsl` + `gdf_populate.wgsl`.
//! Same pattern as the production `SHADER_SOURCE` in `raymarch/mod.rs`
//! and the `POOL_EVAL_SHADER_SOURCE` smoke harness.

#[cfg(feature = "gdf-debug")]
mod debug;
mod state;
mod uniforms;

#[cfg(feature = "gdf-debug")]
pub use debug::GdfDebugCounters;
pub use state::GdfState;
pub use uniforms::{
    CASCADE_0_SIDE_METRES, CASCADE_0_VOXELS_PER_AXIS, CASCADE_0_VOXEL_SIZE, CascadeDescriptor,
    POPULATE_WORKGROUP_XY, snap_to_voxel_grid,
};

/// Concatenated populate compute shader: SDF primitives library +
/// pool traversal library + GDF cascade-sample library (struct
/// declaration only — cascade-fetch eval_scene_bvh is unreachable
/// from `cs_populate` so naga prunes its bindings out of the
/// pipeline layout) + populate entry point. Including the sample
/// library keeps `CascadeDescriptor`'s definition single-sourced
/// so the populate path's group-0 uniform binding shares the layout
/// with the raymarch path's group-1 binding 13.
pub const POPULATE_SHADER_SOURCE: &str = concat!(
    include_str!("../../../ome_sdf/shaders/sdf_primitives.wgsl"),
    "\n",
    include_str!("../../shaders/raymarch_pool_eval.wgsl"),
    "\n",
    include_str!("../../shaders/raymarch_gdf_sample.wgsl"),
    "\n",
    include_str!("../../shaders/gdf_populate.wgsl"),
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn populate_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(POPULATE_SHADER_SOURCE)
            .expect("populate shader must parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("populate shader must validate");
    }

    #[test]
    fn populate_shader_exposes_cs_populate_entry_point() {
        let module = naga::front::wgsl::parse_str(POPULATE_SHADER_SOURCE)
            .expect("populate shader must parse");
        let entry_names: Vec<&str> = module
            .entry_points
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            entry_names.iter().any(|n| *n == "cs_populate"),
            "populate shader must expose `cs_populate`; saw {entry_names:?}"
        );
    }
}
