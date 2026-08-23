use super::*;

/// The jitter moves the count a few pages a frame; that is not news.
///
/// 🔴 The measured flood: `resident` walked 2260, 2268, 2273, 2261
/// with the camera still, because the temporal jitter puts sub-pixel
/// samples in other pages. An equality check calls every one of
/// those a change.
#[test]
fn the_log_ignores_jitter_and_not_a_real_move() {
    for (before, now) in [(2260u32, 2268u32), (2273, 2261), (1669, 1674)] {
        assert!(!moved(before, now), "{before} -> {now} is jitter");
    }
    for (before, now) in [(2260u32, 1600u32), (0, 40), (40, 0), (1024, 512)] {
        assert!(moved(before, now), "{before} -> {now} is news");
    }
}

/// A camera's slot is its own, and asking for a far one grows the
/// list rather than reaching past it.
#[test]
fn a_camera_logs_against_its_own_last() {
    let mut slots: Vec<Option<u32>> = Vec::new();
    *logged(&mut slots, 0) = Some(7);
    *logged(&mut slots, 3) = Some(9);
    assert_eq!(*logged(&mut slots, 0), Some(7));
    assert_eq!(*logged(&mut slots, 1), None);
    assert_eq!(*logged(&mut slots, 3), Some(9));
}

/// The first camera owns the first slice.
///
/// 🔴 A slot map reserves index zero for its null key, so the first
/// real view is index 1 — and a slice numbering that forgot it would
/// leave slice 0 permanently unused and put the last camera one past
/// the end of the pool.
#[test]
fn the_first_view_owns_the_first_slice() {
    let mut views: slotmap::SlotMap<crate::meshlet::render_stage::ViewId, u32> =
        slotmap::SlotMap::with_key();
    let first = views.insert(0);
    let second = views.insert(1);
    assert_eq!(page_view_index(first), 0);
    assert_eq!(page_view_index(second), 1);
    // Destroying and recreating hands the slot back, so a camera's
    // slice is stable rather than a position in an iteration order.
    views.remove(first);
    let third = views.insert(2);
    assert_eq!(page_view_index(third), 0);
}

#[test]
fn no_settings_asset_means_defaults_not_disabled() {
    // 🔴 The half of the bug that is testable without touching the
    // environment. The original read took an early return with a
    // hardcoded `enabled: false` whenever the resource was absent —
    // which was every build. A project with no settings asset is
    // the normal case, so absence has to mean DEFAULTS.
    let resources = Resources::default();
    let settings = page_settings(&resources);
    let defaults = crate::shadow::ShadowSettings::default();
    assert_eq!(settings.density, defaults.page_density);
    assert_eq!(settings.pool.pages, defaults.pool_pages);
}
