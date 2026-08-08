use super::is_engine_owned;

/// The four the engine writes. Two of them are overwritten on the next
/// frame; two of them break something if a second one exists.
#[test]
fn derived_components_are_not_offered() {
    for name in [
        "kooch_ecs::hierarchy::parent::Parent",
        "kooch_ecs::hierarchy::children::Children",
        "kooch_ecs::transform::GlobalTransform",
        "kooch_ecs::persistent_id::PersistentId",
    ] {
        assert!(
            is_engine_owned(name),
            "{name} should not be addable by hand"
        );
    }
}

/// Everything a user actually places has to stay in the menu. This is
/// the assertion that fails if the list is ever widened carelessly.
#[test]
fn authorable_components_stay() {
    for name in [
        "kooch_ecs::transform::Transform",
        "kooch_ecs::name::Name",
        "kooch_ecs::perspective_camera::PerspectiveCamera",
        "kooch_physics::components::PhysicsBody",
        "kooch_camera::virtual_camera::VirtualCamera",
    ] {
        assert!(!is_engine_owned(name), "{name} must remain addable");
    }
}

/// A project's own `Parent` is not the engine's. The prefix check is
/// the whole reason the rule is not just "any type called Parent".
#[test]
fn a_projects_own_type_is_never_caught_by_an_engine_rule() {
    assert!(!is_engine_owned("my_game::hierarchy::Parent"));
    assert!(!is_engine_owned("roll_a_ball::Children"));
}
