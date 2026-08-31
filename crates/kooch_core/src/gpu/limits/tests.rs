//! Test code for `limits`, in its own file.
//!
//! # 🔴 A sibling file, not an inline `mod`
//!
//! The engine vendors its own source into every project, and the
//! walk that copies it skips test code by FILE — it can drop
//! `x_tests.rs` and it cannot reach inside a module written in
//! line. An inline block therefore ships to every game that ever
//! builds against this engine, which is what
//! `the_vendored_engine_contains_no_test_code` is there to catch.

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
