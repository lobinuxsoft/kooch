use wgpu::Adapter;

/// Targets from wgpu capabilities audit §C.1: SSAO tiles (8×8×16 = 1024 invocations)
/// and bloom downsample (~32 KiB LDS) need more than wgpu defaults of 256 / 16 KiB.
/// Clamped against adapter-reported limits so older GPUs fall back gracefully.
pub(super) const TARGET_MAX_COMPUTE_INVOCATIONS_PER_WORKGROUP: u32 = 1024;
pub(super) const TARGET_MAX_COMPUTE_WORKGROUP_STORAGE_SIZE: u32 = 32_768;
/// Hi-Z SPD (#486) binds 12 storage texture slots in one bind
/// group. wgpu's default `max_storage_textures_per_shader_stage` is
/// 4, so the SPD pipeline-layout creation rejects without raising
/// it. Most desktop GPUs (RX 9070 XT included) advertise ≥ 16.
pub(super) const TARGET_MAX_STORAGE_TEXTURES_PER_STAGE: u32 = 16;
/// The atomic R64 visibility-buffer raster pipeline (#493) uses bind
/// groups 0..4 (camera, meshlet pool, visible_meshlets, instances,
/// vbuf64). #454 adds bind group 5 for the triangle-density
/// accumulator + the uniform that gates the atomicAdd in production.
/// wgpu's default `max_bind_groups` is 4 (group indices 0..3), so the
/// BGL creation rejects without raising it. RDNA 2+ desktop /
/// handheld + DX12 + Metal all advertise ≥ 8.
pub(super) const TARGET_MAX_BIND_GROUPS: u32 = 6;
/// The scene-pool atomic cull pipeline already binds 8 storage
/// buffers (params + visible IDs + count + 2 pool descriptors +
/// instances + group_max_err + reject_reasons). #454.6 adds a 9th
/// for per-stage cull survivor counts. wgpu's default
/// `max_storage_buffers_per_shader_stage` is 8 — exactly at the
/// existing budget — so any further cull-side instrumentation must
/// raise it. Most desktop GPUs (RX 9070 XT included) advertise ≥
/// 16; mobile baselines (Snapdragon X Elite / Adreno X1) typically
/// expose 16 too.
pub(super) const TARGET_MAX_STORAGE_BUFFERS_PER_STAGE: u32 = 16;

pub(super) fn elevated_compute_limits(adapter: &Adapter) -> wgpu::Limits {
    let adapter_limits = adapter.limits();

    let invocations = TARGET_MAX_COMPUTE_INVOCATIONS_PER_WORKGROUP
        .min(adapter_limits.max_compute_invocations_per_workgroup);
    let storage = TARGET_MAX_COMPUTE_WORKGROUP_STORAGE_SIZE
        .min(adapter_limits.max_compute_workgroup_storage_size);
    let storage_textures = TARGET_MAX_STORAGE_TEXTURES_PER_STAGE
        .min(adapter_limits.max_storage_textures_per_shader_stage);
    let bind_groups = TARGET_MAX_BIND_GROUPS.min(adapter_limits.max_bind_groups);
    let storage_buffers = TARGET_MAX_STORAGE_BUFFERS_PER_STAGE
        .min(adapter_limits.max_storage_buffers_per_shader_stage);

    if invocations < TARGET_MAX_COMPUTE_INVOCATIONS_PER_WORKGROUP {
        tracing::warn!(
            requested = TARGET_MAX_COMPUTE_INVOCATIONS_PER_WORKGROUP,
            granted = invocations,
            "adapter clamped max_compute_invocations_per_workgroup; compute-heavy passes (SSAO, bloom) may run degraded"
        );
    }
    if storage < TARGET_MAX_COMPUTE_WORKGROUP_STORAGE_SIZE {
        tracing::warn!(
            requested = TARGET_MAX_COMPUTE_WORKGROUP_STORAGE_SIZE,
            granted = storage,
            "adapter clamped max_compute_workgroup_storage_size; tile-based compute passes may need smaller tiles"
        );
    }
    if storage_textures < TARGET_MAX_STORAGE_TEXTURES_PER_STAGE {
        tracing::warn!(
            requested = TARGET_MAX_STORAGE_TEXTURES_PER_STAGE,
            granted = storage_textures,
            "adapter clamped max_storage_textures_per_shader_stage; Hi-Z SPD pyramid build (#486) requires ≥ 12"
        );
    }
    if bind_groups < TARGET_MAX_BIND_GROUPS {
        tracing::warn!(
            requested = TARGET_MAX_BIND_GROUPS,
            granted = bind_groups,
            "adapter clamped max_bind_groups; meshlet pipeline requires ≥ 6 (atomic R64 vbuf #493 + density accumulator #454)"
        );
    }
    if storage_buffers < TARGET_MAX_STORAGE_BUFFERS_PER_STAGE {
        tracing::warn!(
            requested = TARGET_MAX_STORAGE_BUFFERS_PER_STAGE,
            granted = storage_buffers,
            "adapter clamped max_storage_buffers_per_shader_stage; per-stage cull survivor counters (#454.6) need ≥ 9"
        );
    }

    wgpu::Limits {
        max_compute_invocations_per_workgroup: invocations,
        max_compute_workgroup_storage_size: storage,
        max_storage_textures_per_shader_stage: storage_textures,
        max_bind_groups: bind_groups,
        max_storage_buffers_per_shader_stage: storage_buffers,
        ..wgpu::Limits::default()
    }
}
