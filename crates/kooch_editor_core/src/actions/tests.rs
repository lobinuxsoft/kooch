use super::*;

/// The one that motivated the guard: a Save during a build would
/// write the empty mirror over the project's real scene. Not a panic
/// — a deleted scene.
#[test]
fn saving_needs_a_live_world() {
    assert!(EditorAction::SaveScene.needs_a_live_world());
}

/// Every escape hatch has to keep working, or a build that never
/// finishes leaves the editor with no way out.
#[test]
fn the_ways_out_of_a_stuck_build_are_not_blocked() {
    for (name, action) in [
        ("CancelLaunch", EditorAction::CancelLaunch),
        ("RebuildRemote", EditorAction::RebuildRemote),
        ("CloseProject", EditorAction::CloseProject),
    ] {
        assert!(
            !action.needs_a_live_world(),
            "{name} is how a user recovers; blocking it traps them",
        );
    }
}

/// Play asks the *project* to start simulating. Sent before it can
/// answer, it is a message into a socket nobody is reading yet.
#[test]
fn play_and_stop_wait_for_the_project() {
    assert!(EditorAction::Play.needs_a_live_world());
    assert!(EditorAction::Stop.needs_a_live_world());
}

/// An asset edit is about a file on disk, and the project is not
/// holding it — refusing these would block work that is perfectly
/// safe during a build.
#[test]
fn preferences_and_file_work_stay_available() {
    assert!(!EditorAction::SetPowerProfile(PowerProfile::Battery).needs_a_live_world());
}
