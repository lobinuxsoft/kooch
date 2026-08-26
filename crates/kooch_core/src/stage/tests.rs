use super::*;

#[test]
fn stage_ordering() {
    assert!(Stage::Startup < Stage::First);
    assert!(Stage::First < Stage::Update);
    assert!(Stage::Update < Stage::Physics);
    assert!(Stage::Physics < Stage::Render);
    assert!(Stage::Render < Stage::Last);
}

#[test]
fn fixed_stages() {
    assert!(Stage::Physics.is_fixed());
    assert!(Stage::PostPhysics.is_fixed());
    assert!(!Stage::Update.is_fixed());
    assert!(!Stage::Render.is_fixed());
}

#[test]
fn startup_stage() {
    assert!(Stage::Startup.is_startup());
    assert!(!Stage::First.is_startup());
}

#[test]
fn all_stages_count() {
    assert_eq!(Stage::ALL.len(), 14);
}

#[test]
fn stage_names() {
    assert_eq!(Stage::Startup.name(), "Startup");
    assert_eq!(Stage::Physics.name(), "Physics");
    assert_eq!(Stage::Last.name(), "Last");
}

#[test]
fn display_impl() {
    assert_eq!(format!("{}", Stage::Update), "Update");
}
