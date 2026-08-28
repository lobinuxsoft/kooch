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
///
/// 🔴 **This budget is now fully spent.** The two-pass material shading
/// pipeline (#440) uses groups 0..4 and #441 put Inti's lights on
/// group 5 — six, exactly the target. There is no seventh group to
/// hand out.
///
/// That is a constraint on the shadow work, not a number to raise on
/// reflex: **shadow maps belong in Inti's group**, next to the lights
/// that cast them, because a shadow map without its light is not a
/// thing any shader wants. Raising the target to 8 would work on this
/// hardware and quietly drop the baseline the engine claims to
/// support — Vulkan only guarantees 4.
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

/// The largest a SINGLE buffer may be, and the largest a single storage
/// BINDING may be. Both are wgpu's conservative defaults, restated here
/// on purpose.
///
/// # 🔴 Declared rather than inherited
///
/// Every other limit in this file is named, clamped and warned about
/// when an adapter grants less. These two used to fall through to
/// `Limits::default()`, so the engine ran with 256 MiB / 128 MiB
/// ceilings that nothing stated and nothing logged.
///
/// That is not academic. A buffer past either is **not an error at
/// creation**: wgpu returns an INVALID buffer, and every submit
/// afterwards fails validation with a message that names a label and
/// no cause. A 2.4 GB lamp-cull arena looked, from the log, like a
/// wall of identical lines with nothing to grep for.
///
/// # ⚠️ These are PER BUFFER, not a budget
///
/// Forty buffers of 200 MiB are fine; one of 300 MiB is not. Nothing
/// here caps total VRAM.
///
/// # ⚠️ And they are kept at the FLOOR deliberately
///
/// An RX 9070 XT offers 2048 MiB for both — eight times this. Taking
/// it would let a buffer through here that the OneXFly cannot hold,
/// and the failure would surface on the handheld, over SSH, in a build
/// nobody wants to make twice. The portable floor is the useful number;
/// what was missing was saying so out loud.
pub(super) const TARGET_MAX_BUFFER_SIZE: u64 = 256 << 20;
pub(super) const TARGET_MAX_STORAGE_BUFFER_BINDING_SIZE: u64 = 128 << 20;

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
    let buffer_size = TARGET_MAX_BUFFER_SIZE.min(adapter_limits.max_buffer_size);
    let binding_size =
        TARGET_MAX_STORAGE_BUFFER_BINDING_SIZE.min(adapter_limits.max_storage_buffer_binding_size);

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

    if buffer_size < TARGET_MAX_BUFFER_SIZE {
        tracing::warn!(
            requested_mib = TARGET_MAX_BUFFER_SIZE / (1 << 20),
            granted_mib = buffer_size / (1 << 20),
            "adapter clamped max_buffer_size; a buffer past it is not rejected at creation — wgpu returns an INVALID buffer and every later submit fails naming a label and no cause"
        );
    }
    if binding_size < TARGET_MAX_STORAGE_BUFFER_BINDING_SIZE {
        tracing::warn!(
            requested_mib = TARGET_MAX_STORAGE_BUFFER_BINDING_SIZE / (1 << 20),
            granted_mib = binding_size / (1 << 20),
            "adapter clamped max_storage_buffer_binding_size; the page table, the mesh pool and the cull arenas all bind as storage"
        );
    }
    // 🟢 The other direction, and it is information rather than a
    // problem: an adapter that offers more than the floor is a machine
    // this build will NOT use the headroom of, on purpose. Said once,
    // at debug, so a capture taken here is read against the right
    // ceiling — and so nobody concludes from a desktop that a buffer
    // size is safe.
    if adapter_limits.max_buffer_size > TARGET_MAX_BUFFER_SIZE {
        tracing::debug!(
            offered_mib = adapter_limits.max_buffer_size / (1 << 20),
            using_mib = TARGET_MAX_BUFFER_SIZE / (1 << 20),
            "adapter offers more buffer headroom than the engine asks for; the floor is the portable one"
        );
    }

    wgpu::Limits {
        max_compute_invocations_per_workgroup: invocations,
        max_compute_workgroup_storage_size: storage,
        max_storage_textures_per_shader_stage: storage_textures,
        max_bind_groups: bind_groups,
        max_storage_buffers_per_shader_stage: storage_buffers,
        max_buffer_size: buffer_size,
        max_storage_buffer_binding_size: binding_size,
        ..wgpu::Limits::default()
    }
}
