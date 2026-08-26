//! The flag that decides whether the mirror runs.
//!
//! Getting it wrong in the cheap direction wastes 7.5 ms a frame.
//! Getting it wrong in the expensive direction leaves the editor
//! showing a world the project no longer has — so the cases that
//! must report "changed" are the ones worth pinning down.

/// Mirrors the decision made in `refresh`, without a project to talk
/// to. Kept in step with the call site by being one expression.
fn changed(full: bool, entities: usize, removed: usize) -> bool {
    full || entities != 0 || removed != 0
}

#[test]
fn an_empty_diff_reports_no_change() {
    assert!(!changed(false, 0, 0));
}

#[test]
fn a_changed_entity_reports_a_change() {
    assert!(changed(false, 1, 0));
}

/// A despawn carries no entities at all — only an id in `removed`.
/// Missing it would leave the deleted entity on screen.
#[test]
fn a_removal_alone_reports_a_change() {
    assert!(changed(false, 0, 1));
}

/// A full reply arrives exactly when the project could not honour
/// our revision, so what we hold cannot be trusted to match — even
/// if the reply happens to be empty, which is what a full reply for
/// an empty world looks like.
#[test]
fn a_full_reply_always_reports_a_change() {
    assert!(changed(true, 0, 0));
}
