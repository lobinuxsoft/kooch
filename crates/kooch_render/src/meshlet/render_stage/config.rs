use crate::meshlet::MeshletDebugCaps;
use crate::vbuf64::Vbuf64Support;

/// Construction parameters for `MeshletRenderStage`. All sizes are upper bounds — the actual
/// per-frame instance count comes from the ECS query.
#[derive(Debug, Clone, Copy)]
pub struct MeshletRenderStageConfig {
    /// Output color / depth / vbuf resolution. Must be > 0 in both axes.
    pub size: (u32, u32),
    /// Maximum number of `MeshInstance`
    /// records the scene buffer can hold per frame.
    pub instance_capacity: u32,
    /// Capacity (in surviving meshlet slots) of the cull dispatcher's `visible_meshlets` storage.
    /// For the scene path, set this to at least `instance_capacity * meshlets_per_mesh` so no
    /// thread loses its slot to atomic-overflow.
    pub meshlet_capacity: u32,
    /// Runtime decision of whether the atomic R64 visibility-buffer path (#493) is available.
    pub vbuf64: Vbuf64Support,
    /// Capability probe (#454) for the advanced debug modes.
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
