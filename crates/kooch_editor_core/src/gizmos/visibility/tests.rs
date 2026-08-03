//! Tests for the visibility model and its grouping.

use super::*;

const COLLIDER: &str = "kooch_physics::components::Collider";
const CAMERA: &str = "kooch_ecs::perspective_camera::PerspectiveCamera";

#[test]
fn everything_draws_by_default() {
    let v = GizmoVisibility::new();
    assert!(v.draws(COLLIDER, Some("Physics")));
    assert!(v.draws(CAMERA, Some("Rendering")));
    assert!(v.draws("game::Thing", None));
    assert!(!v.has_exceptions());
}

/// A component with no entry draws. That is what lets a visualizer
/// added later appear without registering anything here, and without
/// migrating anyone's saved settings.
#[test]
fn an_unknown_component_draws() {
    let mut v = GizmoVisibility::new();
    v.set_category("Physics", false);
    assert!(
        v.draws("brand::New::Component", Some("Lighting")),
        "an unrelated new component was hidden"
    );
}

#[test]
fn hiding_a_category_hides_only_its_own() {
    let mut v = GizmoVisibility::new();
    v.set_category("Physics", false);

    assert!(!v.draws(COLLIDER, Some("Physics")));
    assert!(v.draws(CAMERA, Some("Rendering")));
    assert!(v.has_exceptions());
}

#[test]
fn hiding_one_component_leaves_its_category_alone() {
    let mut v = GizmoVisibility::new();
    v.set_component(COLLIDER, false);

    assert!(!v.draws(COLLIDER, Some("Physics")));
    assert!(
        v.draws("kooch_physics::components::PhysicsBody", Some("Physics")),
        "hiding one component hid its whole category"
    );
    assert!(v.category_visible("Physics"));
}

/// The master switch hides everything and restores exactly what was
/// set — the point of keeping it separate from the per-group state.
#[test]
fn the_master_switch_preserves_the_per_group_state() {
    let mut v = GizmoVisibility::new();
    v.set_category("Physics", false);

    v.enabled = false;
    assert!(!v.draws(CAMERA, Some("Rendering")));
    assert!(!v.draws(COLLIDER, Some("Physics")));

    v.enabled = true;
    assert!(v.draws(CAMERA, Some("Rendering")), "restoring lost a group");
    assert!(
        !v.draws(COLLIDER, Some("Physics")),
        "restoring forgot the hidden category"
    );
}

#[test]
fn show_all_clears_every_exception() {
    let mut v = GizmoVisibility::new();
    v.set_category("Physics", false);
    v.set_component(CAMERA, false);
    v.enabled = false;

    v.show_all();

    assert!(v.draws(COLLIDER, Some("Physics")));
    assert!(v.draws(CAMERA, Some("Rendering")));
    assert!(!v.has_exceptions());
}

/// Choices have to survive a restart, so they round-trip through the
/// same format the dock layout uses.
#[test]
fn visibility_round_trips_through_ron() {
    let mut v = GizmoVisibility::new();
    v.set_category("Physics", false);
    v.set_component(CAMERA, false);

    let text = ron::ser::to_string(&v).expect("serialise");
    let back: GizmoVisibility = ron::from_str(&text).expect("deserialise");

    assert!(!back.draws(COLLIDER, Some("Physics")));
    assert!(!back.draws(CAMERA, Some("Rendering")));
    assert!(back.draws("kooch_ecs::point_light::PointLight", Some("Lighting")));
}

/// An older saved layout has no `enabled` field; it must read as on
/// rather than hiding every gizmo the user had.
#[test]
fn an_older_saved_layout_reads_as_visible() {
    let back: GizmoVisibility =
        ron::from_str("(hidden_categories: [], hidden_components: [])").expect("deserialise");
    assert!(back.enabled, "a layout without the field hid everything");
    assert!(back.draws(COLLIDER, Some("Physics")));
}

#[test]
fn grouping_sorts_categories_and_leaves_uncategorised_last() {
    let groups = group_visualizers([
        (TypeId::of::<u8>(), "game::Thing".to_owned(), None),
        (
            TypeId::of::<u16>(),
            COLLIDER.to_owned(),
            Some("Physics".to_owned()),
        ),
        (
            TypeId::of::<u32>(),
            CAMERA.to_owned(),
            Some("Rendering".to_owned()),
        ),
        (
            TypeId::of::<u64>(),
            "kooch_physics::components::PhysicsBody".to_owned(),
            Some("Physics".to_owned()),
        ),
    ]);

    let names: Vec<Option<&str>> = groups.iter().map(|g| g.category.as_deref()).collect();
    assert_eq!(names, vec![Some("Physics"), Some("Rendering"), None]);

    // Physics holds both of its components, sorted by short name.
    let physics = &groups[0].components;
    assert_eq!(
        physics.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(),
        vec!["Collider", "PhysicsBody"]
    );
}
