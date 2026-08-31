//! Test code for `pool`, in its own file.
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

fn pool(pages: u32, views: u32) -> PoolConfig {
    PoolConfig {
        pages,
        views,
        row_cap: u32::MAX,
    }
}

/// The exact shape that shipped a blue screen: the same pool the
/// editor renders across two views does not fit the game's one.
#[test]
fn one_view_is_the_case_two_views_hid() {
    assert_eq!(pool(6144, 2).per_row() * 128, 7168, "two views fit");
    assert_eq!(pool(6144, 1).per_row() * 128, 10112, "one view does not");
}

/// 🔴 The budget survives the cap. The clamp this replaces bought
/// 4096 of the 6144 asked for; the layers keep all of them.
#[test]
fn the_cap_multiplies_layers_not_lost_pages() {
    let capped = pool(6144, 1).fit_atlas(8192, 128);
    assert_eq!(capped.per_row(), 64, "a layer stops at the limit");
    assert_eq!(capped.per_row() * 128, 8192);
    assert_eq!(capped.slice(), 4096, "pages a layer holds");
    assert_eq!(capped.layers_per_view(), 2);
    assert_eq!(capped.layers(), 2, "the atlas grew in depth");
    assert!(
        capped.slots() >= 6144,
        "kept the budget: {}",
        capped.slots()
    );
}

/// A pool that already fits gains nothing and loses nothing.
#[test]
fn a_pool_that_fits_stays_one_layer() {
    let small = pool(1024, 1).fit_atlas(8192, 128);
    assert_eq!(small.layers_per_view(), 1);
    assert_eq!(small.layers(), 1);
    assert_eq!(small.slots(), small.slice());
}

/// Two views keep one layer each, so the editor's atlas is the
/// shape it always was.
#[test]
fn two_views_keep_a_layer_each() {
    let two = pool(6144, 2).fit_atlas(8192, 128);
    assert_eq!(two.layers_per_view(), 1);
    assert_eq!(two.layers(), 2);
}

/// 🔴 Slots are global and a view's are contiguous, which is what
/// lets `slot / slice` name a layer without knowing whose it is.
#[test]
fn a_views_slots_are_its_own_layers() {
    let p = pool(6144, 2).fit_atlas(8192, 128);
    assert_eq!(p.base(0), 0);
    assert_eq!(p.base(1), p.slots());
    // The layer a view's first and last slot land in.
    assert_eq!(p.base(1) / p.slice(), p.layers_per_view());
}
