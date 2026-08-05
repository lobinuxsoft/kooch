use crate::meshlet::MeshletDebugCaps;
use crate::vbuf64::Vbuf64Support;

/// Construction parameters for [`MeshletRenderStage`]. All sizes are
/// upper bounds — the actual per-frame instance count comes from the
/// ECS query.
///
/// The stage keeps a copy so `create_view` can build additional views
/// with the same capabilities. Two views of one stage disagreeing about
/// whether the atomic path exists would be a bug with no honest cause.
#[derive(Debug, Clone, Copy)]
pub struct MeshletRenderStageConfig {
    /// Output color / depth / vbuf resolution. Must be > 0 in both axes.
    pub size: (u32, u32),
    /// Maximum number of [`MeshInstance`](super::scene::MeshInstance)
    /// records the scene buffer can hold per frame.
    pub instance_capacity: u32,
    /// Capacity (in surviving meshlet slots) of the cull dispatcher's
    /// `visible_meshlets` storage. For the scene path, set this to at
    /// least `instance_capacity * meshlets_per_mesh` so no thread loses
    /// its slot to atomic-overflow.
    pub meshlet_capacity: u32,
    /// Runtime decision of whether the atomic R64 visibility-buffer
    /// path (#493) is available. When `is_supported()` returns true
    /// the stage owns a [`Vbuf64Stage`] alongside the legacy R32Uint
    /// resources and the per-frame orchestrator picks the atomic
    /// path; otherwise only the R32Uint path is built.
    pub vbuf64: Vbuf64Support,
    /// Capability probe (#454) for the advanced debug modes. When
    /// `supports_texture_atomic()` is true the stage allocates a
    /// per-pixel R32Uint atomic accumulator backing TriangleDensity /
    /// Overdraw / reject overlays; otherwise the texture stays `None`
    /// and the dropdown hides the dependent modes.
    pub debug_caps: MeshletDebugCaps,
}

impl Default for MeshletRenderStageConfig {
    fn default() -> Self {
        Self {
            size: (256, 256),
            instance_capacity: 256,
            meshlet_capacity: 4096,
            vbuf64: Vbuf64Support::from_supported(false),
            debug_caps: MeshletDebugCaps::default(),
        }
    }
}
