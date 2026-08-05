//! Construction: `MeshletCullPipelines::new` (shared pipelines +
//! layouts) and `MeshletCull::new` (one view's buffers).

mod bgls;
mod new;
mod pipelines;

const CULL_SHADER_SOURCE: &str = concat!(
    include_str!("../../../../shaders/meshlet_cull/common.wgsl"),
    include_str!("../../../../shaders/meshlet_cull/basic.wgsl"),
    include_str!("../../../../shaders/meshlet_cull/scene.wgsl"),
    include_str!("../../../../shaders/meshlet_cull/pool.wgsl"),
    include_str!("../../../../shaders/meshlet_cull/atomic.wgsl"),
    include_str!("../../../../shaders/meshlet_cull/atomic_hi_z.wgsl"),
);
