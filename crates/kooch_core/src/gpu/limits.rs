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
/// How many workgroups one `dispatch_workgroups` dimension may hold.
///
/// # 🔴 The ceiling is on the DISPATCH, not on the work
///
/// At 64 threads per group this dimension tops out at **4 194 240
/// threads**. A cull that runs one thread per (instance × meshlet)
/// reaches that at, say, 846 copies of a 4 953-meshlet dragon — an
/// unremarkable open-world scene, not a stress test.
///
/// Past it the dispatch is **rejected outright**: the whole encoder
/// fails validation and the frame draws nothing, once per frame,
/// forever. A dense scene reported it as `[156639, 1, 1] must be less
/// or equal to 65535` and nothing else — no hint that the count came
/// from a cull, or which one.
///
/// # The fix is to fold, not to cap
///
/// [`tiled_workgroups`] spills the excess into a second dimension and
/// the shader re-linearises it from `num_workgroups`. Clamping instead
/// would silently stop culling past the 4.2 M-th meshlet, which is
/// worse than crashing: the scene would render, missing geometry, and
/// look like a bug in the LOD chain.
///
/// # ⚠️ Every backend guarantees exactly this and no more
///
/// 65 535 is the Vulkan / D3D12 / Metal floor and also what desktop
/// adapters actually report — the RX 9070 XT included. Unlike the
/// buffer sizes there is no headroom to leave on the table here.
pub const MAX_WORKGROUPS_PER_DIM: u32 = 65_535;

/// Split `threads` into a 2-D workgroup count that no dimension
/// overflows, given a 1-D `workgroup_size`.
///
/// Returns `(x, y)` for `dispatch_workgroups(x, y, 1)`. Below the
/// ceiling `y` is 1 and the dispatch is the plain 1-D one; above it
/// `x` saturates and `y` carries the rest, so the shader recovers its
/// linear index as `gid.y * (num_workgroups.x * workgroup_size) +
/// gid.x`.
///
/// The tiled form over-covers — the last row runs threads past
/// `threads`. Every caller already guards on its own count, which is
/// why this returns the shape and not the bound.
pub fn tiled_workgroups(threads: u32, workgroup_size: u32) -> (u32, u32) {
    debug_assert!(workgroup_size > 0, "a workgroup cannot be empty");
    let groups = threads.div_ceil(workgroup_size.max(1)).max(1);
    if groups <= MAX_WORKGROUPS_PER_DIM {
        (groups, 1)
    } else {
        (
            MAX_WORKGROUPS_PER_DIM,
            groups.div_ceil(MAX_WORKGROUPS_PER_DIM),
        )
    }
}

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
    let workgroups_per_dim =
        MAX_WORKGROUPS_PER_DIM.min(adapter_limits.max_compute_workgroups_per_dimension);

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
    if workgroups_per_dim < MAX_WORKGROUPS_PER_DIM {
        // 🔴 Not a graceful degradation. `tiled_workgroups` saturates a
        // dimension at MAX_WORKGROUPS_PER_DIM, so an adapter below it
        // rejects the very dispatch the tiling exists to make legal.
        // 65 535 is the floor Vulkan, D3D12 and Metal all guarantee;
        // reaching this branch means the assumption the tiling rests on
        // is false on this machine, and it should be said that way.
        tracing::warn!(
            requested = MAX_WORKGROUPS_PER_DIM,
            granted = workgroups_per_dim,
            "adapter reports fewer workgroups per dimension than every backend guarantees; tiled cull dispatches will be rejected on this device"
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
        max_compute_workgroups_per_dimension: workgroups_per_dim,
        ..wgpu::Limits::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the shader reverses: `gid.y * (x * size) + gid.x`.
    fn covered(threads: u32, size: u32) -> u64 {
        let (x, y) = tiled_workgroups(threads, size);
        u64::from(x) * u64::from(y) * u64::from(size)
    }

    #[test]
    fn a_small_count_stays_one_dimensional() {
        assert_eq!(tiled_workgroups(64 * 100, 64), (100, 1));
        // Zero work still dispatches one group; the shader's own bound
        // check discards it. Returning (0, ..) would be a no-op the
        // callers do not expect.
        assert_eq!(tiled_workgroups(0, 64), (1, 1));
    }

    #[test]
    fn the_last_one_dimensional_count_is_exact() {
        let threads = MAX_WORKGROUPS_PER_DIM * 64;
        assert_eq!(tiled_workgroups(threads, 64), (MAX_WORKGROUPS_PER_DIM, 1));
        assert_eq!(tiled_workgroups(threads + 1, 64).1, 2);
    }

    /// 2024 dragons × 4953 meshlets — the dense scene that found this.
    /// A 1-D dispatch asks for 156 639 groups and wgpu rejects the whole
    /// encoder; the fold has to cover the count without exceeding the
    /// ceiling in either dimension.
    #[test]
    fn the_dense_scene_fits_in_two_dimensions() {
        let threads = 2024u32 * 4953;
        let (x, y) = tiled_workgroups(threads, 64);
        assert!(x <= MAX_WORKGROUPS_PER_DIM, "x overflows: {x}");
        assert!(y <= MAX_WORKGROUPS_PER_DIM, "y overflows: {y}");
        assert!(covered(threads, 64) >= u64::from(threads));
    }

    /// Over-covering is fine — every `run_*` guards on its own total —
    /// but UNDER-covering silently drops meshlets, which renders as
    /// missing geometry and reads like an LOD bug.
    #[test]
    fn no_count_is_left_uncovered() {
        for threads in [1, 63, 65, 4_194_240, 4_194_241, 10_024_872, u32::MAX] {
            assert!(
                covered(threads, 64) >= u64::from(threads),
                "{threads} threads under-covered"
            );
        }
    }
}
