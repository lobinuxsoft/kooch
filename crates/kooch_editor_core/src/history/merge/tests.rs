use super::*;

/// The same field edited again continues the same step. This is the
/// difference between one Ctrl+Z and sixty.
#[test]
fn the_same_target_continues() {
    let key = MergeKey::of(("entity", 3u32, "Transform", "position"));
    assert!(continues(Some(key), Some(key), false));
}

/// A different field starts a new one, even mid-drag.
#[test]
fn another_target_starts_over() {
    let position = MergeKey::of(("entity", 3u32, "Transform", "position"));
    let scale = MergeKey::of(("entity", 3u32, "Transform", "scale"));
    assert!(!continues(Some(position), Some(scale), false));
}

/// 🔴 The seal is what stops a field edited now from merging with the
/// same field edited an hour ago. Without it the two would be one step,
/// and undoing the second would silently undo the first.
#[test]
fn a_seal_ends_the_run() {
    let key = MergeKey::of("same");
    assert!(!continues(Some(key), Some(key), true));
}

/// An edit with no key is discrete: spawning twice is two steps however
/// fast the clicks were.
#[test]
fn keyless_edits_never_merge() {
    let key = MergeKey::of("same");
    assert!(!continues(None, None, false));
    assert!(!continues(Some(key), None, false));
    assert!(!continues(None, Some(key), false));
}
