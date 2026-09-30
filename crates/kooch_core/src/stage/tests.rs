use super::*;

/// 🔴 Discriminants, NOT the frame's order — `PostUpdate`..`Gpu` are numbered before the fixed
/// stages and `PrePhysics` is numbered last, so this says nothing about when anything runs. What
/// it guards is that the numbers never move, because a dynamic plugin sends the one it was built
/// with. `the_frame_runs_prephysics_first` is the test about order.
#[test]
fn the_discriminants_hold() {
    assert_eq!(Stage::Startup as u8, 0);
    assert_eq!(Stage::Update as u8, 4);
    assert_eq!(Stage::Physics as u8, 8);
    assert_eq!(Stage::Last as u8, 13);
    assert_eq!(Stage::PrePhysics as u8, 14);
}

#[test]
fn all_stages_count() {
    assert_eq!(Stage::ALL.len(), 15);
}

#[test]
fn every_stage_is_named() {
    let mut names: Vec<&str> = Stage::ALL.iter().map(|stage| stage.name()).collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), total, "two stages answer to the same name");
    assert_eq!(Stage::PrePhysics.name(), "PrePhysics");
}

#[test]
fn display_impl() {
    assert_eq!(format!("{}", Stage::Update), "Update");
}
