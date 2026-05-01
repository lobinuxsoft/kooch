//! Shared building blocks for the Karras LBVH GPU pipelines.
//!
//! Both the BLAS pipeline (per-chunk leaf AABBs, [`crate::gpu::lbvh`])
//! and the TLAS pipeline (chunk-descriptor centres, introduced in
//! epic #370 PR-1) use the same Karras 2012 algorithm with the same
//! workgroup size, the same uniform layout, and the same AABB
//! propagation iteration count. Centralising those primitives here
//! lets the two pipelines diverge only on input buffer shape — the
//! algorithmic core stays in lockstep.

/// Workgroup size for every Karras compute pass — leaves, internal
/// node construction, and AABB propagation. Matches the
/// `@workgroup_size(64)` declarations in every `karras_*.wgsl` file.
/// 64 is the AMD wavefront size and an even divisor of NVIDIA / Intel
/// subgroup widths — portable choice with no per-vendor tuning.
pub const KARRAS_WORKGROUP_SIZE: u32 = 64;

/// Additive slack on top of the `2 × log_n` multiplicative factor in
/// [`aabb_iterations`]. **EMPIRICAL — not a tight theoretical bound.**
///
/// Karras 2012 proves `depth ≤ ⌈log₂ N⌉` for sorted Morton inputs with
/// strict ordering, but in practice random AABBs at `N >> 1024` produce
/// sub-trees where the balance proof's small constants matter: at
/// `N = 65 000` random Karras, the previous `log_n + 4` budget left
/// the root unconverged. The current `2 × log_n + 4` formula clears
/// every input we exercise in the golden suite up to `N = 65 000`.
///
/// **At `N = 1 M+` an adversarial topology may still exceed this bound
/// silently and produce wrong-but-plausible AABBs.** The
/// `cfg(debug_assertions)` invariant check in
/// [`crate::gpu::build`] catches that case (panics with the offending
/// `N`); release builds skip the check for zero overhead. The
/// definitive fix — a single-dispatch atomic-counter bottom-up à la
/// Karras' CUDA implementation — needs WGSL primitives that don't
/// exist portably yet (cross-workgroup memory model + subgroup ops
/// stable across RDNA / Ada / Apple). Tracked as a follow-up; see
/// `BUG 3` in the project memory for the full rationale.
pub(crate) const AABB_ITERATION_SLACK: u32 = 4;

/// Uniform configuration shared by every Karras pass. Only the leaf
/// count `n` is dynamic; padding completes the 16-byte std140/std430
/// slot so the WGSL `LbvhConfig` (BLAS) and `TlasConfig` (TLAS)
/// structs can be byte-identical and the same Rust struct can back
/// both buffers.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub struct KarrasConfig {
    pub n: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

/// Number of times the AABB propagation pass must be dispatched for a
/// tree of `n` leaves. Returns `0` for `n <= 1` (no internals).
///
/// See [`AABB_ITERATION_SLACK`] for the rationale behind the
/// `2 × log_n + 4` formula and the limits of the empirical bound.
pub fn aabb_iterations(n: u32) -> u32 {
    if n <= 1 {
        return 0;
    }
    // Bits required to represent (n - 1) — i.e. ⌈log₂ n⌉.
    let log_n = 32 - (n - 1).leading_zeros();
    2 * log_n + AABB_ITERATION_SLACK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_iterations_grows_with_log_n() {
        assert_eq!(aabb_iterations(0), 0);
        assert_eq!(aabb_iterations(1), 0);
        // 2 × ⌈log₂ N⌉ + 4 slack.
        assert_eq!(aabb_iterations(2), 2 * 1 + AABB_ITERATION_SLACK);
        assert_eq!(aabb_iterations(8), 2 * 3 + AABB_ITERATION_SLACK);
        assert_eq!(aabb_iterations(1024), 2 * 10 + AABB_ITERATION_SLACK);
        assert_eq!(aabb_iterations(65536), 2 * 16 + AABB_ITERATION_SLACK);
    }

    #[test]
    fn karras_config_is_16_bytes() {
        // WGSL std140/std430 uniform slot — must stay 16 bytes so the
        // BLAS + TLAS shaders can share a single uniform layout.
        assert_eq!(std::mem::size_of::<KarrasConfig>(), 16);
    }
}
