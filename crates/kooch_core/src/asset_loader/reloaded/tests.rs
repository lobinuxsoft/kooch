use super::ReloadedAssets;
use crate::Guid;

#[test]
fn an_unwritten_asset_is_at_zero() {
    assert_eq!(ReloadedAssets::new().revision(Guid::new_v4()), 0);
}

#[test]
fn writing_moves_the_revision() {
    let mut reloaded = ReloadedAssets::new();
    let guid = Guid::new_v4();
    reloaded.bump(guid);
    assert_eq!(reloaded.revision(guid), 1);
    reloaded.bump(guid);
    assert_eq!(reloaded.revision(guid), 2);
}

#[test]
fn one_asset_does_not_move_another() {
    let mut reloaded = ReloadedAssets::new();
    let (first, second) = (Guid::new_v4(), Guid::new_v4());
    reloaded.bump(first);
    assert_eq!(reloaded.revision(second), 0);
}

#[test]
fn reading_does_not_consume() {
    // 🔴 Two consumers derive things from one asset — a block's render
    // mesh and its collider. A queue would let whichever drained first
    // hide the change from the other.
    let mut reloaded = ReloadedAssets::new();
    let guid = Guid::new_v4();
    reloaded.bump(guid);
    assert_eq!(reloaded.revision(guid), 1);
    assert_eq!(reloaded.revision(guid), 1);
}
