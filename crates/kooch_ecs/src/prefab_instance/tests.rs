use super::*;

fn address(entity: usize, component: &str, field: &str) -> OverrideAddress {
    OverrideAddress {
        entity,
        component: component.to_owned(),
        field: field.to_owned(),
    }
}

#[test]
fn an_instance_starts_with_nothing_overridden() {
    let instance = PrefabInstance::new(Guid::new_v4());
    assert!(instance.addresses().is_empty());
}

#[test]
fn marking_then_reading_round_trips() {
    let mut instance = PrefabInstance::new(Guid::new_v4());
    let moved = address(0, "kooch_ecs::transform::Transform", "position");
    instance.mark(moved.clone(), None);
    assert_eq!(instance.addresses(), vec![moved.clone()]);
    assert!(instance.is_overridden(&moved));
}

/// Dragging a gizmo emits an edit per drag; marking the same field
/// twice must not grow the set, or a long session's scene file fills
/// with the same address.
#[test]
fn marking_the_same_field_twice_records_it_once() {
    let mut instance = PrefabInstance::default();
    let moved = address(0, "T", "position");
    instance.mark(moved.clone(), None);
    instance.mark(moved, None);
    assert_eq!(instance.addresses().len(), 1);
}

/// The whole point of recording: reverting has something to remove.
/// A diff would have nothing to revert *to*.
#[test]
fn reverting_drops_only_the_field_named() {
    let mut instance = PrefabInstance::default();
    let position = address(0, "T", "position");
    let scale = address(0, "T", "scale");
    instance.mark(position.clone(), None);
    instance.mark(scale.clone(), None);

    instance.revert(&position);
    assert!(!instance.is_overridden(&position));
    assert!(
        instance.is_overridden(&scale),
        "an unrelated field was reverted"
    );
}

#[test]
fn reverting_everything_leaves_nothing() {
    let mut instance = PrefabInstance::default();
    instance.mark(address(0, "T", "position"), None);
    instance.mark(address(1, "U", "health"), None);
    instance.revert_all();
    assert!(instance.addresses().is_empty());
}

/// Two instances in the same state must produce the same bytes, or
/// re-saving a scene shows a diff where nothing changed.
#[test]
fn the_encoding_does_not_depend_on_the_order_marks_arrived_in() {
    let mut first = PrefabInstance::default();
    first.mark(address(1, "B", "y"), None);
    first.mark(address(0, "A", "x"), None);

    let mut second = PrefabInstance::default();
    second.mark(address(0, "A", "x"), None);
    second.mark(address(1, "B", "y"), None);

    assert_eq!(first.overrides, second.overrides);
}

/// A hand-edited scene should cost the overrides it corrupted, not the
/// instance's link to its prefab.
#[test]
fn a_malformed_record_is_skipped_rather_than_poisoning_the_set() {
    let mut instance = PrefabInstance::new(Guid::new_v4());
    instance.mark(address(0, "T", "position"), None);
    instance
        .overrides
        .push_str("\u{1e}garbage\u{1e}also\u{1f}bad");

    assert_eq!(instance.addresses().len(), 1);
    assert!(instance.source.is_some(), "the link survived");
}

/// Component type paths contain `::` and names can contain most
/// things; the separators must not be something a real address holds.
#[test]
fn a_realistic_address_survives_the_encoding() {
    let mut instance = PrefabInstance::default();
    let real = address(
        3,
        "kooch_render::mesh_renderer::MeshRenderer",
        "cast_shadows",
    );
    instance.mark(real.clone(), None);
    assert_eq!(instance.addresses(), vec![real]);
}
