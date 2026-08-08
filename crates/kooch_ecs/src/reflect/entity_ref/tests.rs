use super::*;

fn guid(raw: u64) -> EntityGuid {
    EntityGuid::new(raw).expect("non-zero")
}

#[test]
fn a_live_reference_yields_its_entity_and_no_identity() {
    let entity = Entity::new(7, 2);
    let reference = EntityRef::live(entity);
    assert_eq!(reference.entity(), Some(entity));
    assert_eq!(reference.persistent_id(), None);
    assert!(!reference.is_unresolved());
}

#[test]
fn a_persistent_reference_yields_its_identity_and_no_entity() {
    let reference = EntityRef::same_scene(guid(9));
    assert_eq!(reference.entity(), None);
    assert_eq!(reference.persistent_id(), Some(guid(9)));
    assert!(reference.is_unresolved());
}

/// An internal reference must not name its own scene, or copying the
/// scene would carry references back to the original.
#[test]
fn a_same_scene_reference_names_no_scene() {
    assert_eq!(EntityRef::same_scene(guid(1)).scene(), None);
}

#[test]
fn a_cross_scene_reference_keeps_its_scene() {
    let scene = Guid::new_v4();
    assert_eq!(EntityRef::in_scene(scene, guid(1)).scene(), Some(scene));
}

/// A same-scene reference must not serialise a `scene: None` field —
/// every internal link in every scene file would carry it.
#[test]
fn a_same_scene_reference_omits_the_scene_field() {
    let encoded = ron::to_string(&EntityRef::same_scene(guid(3))).expect("serialises");
    assert!(
        !encoded.contains("scene"),
        "same-scene refs must stay terse, got {encoded}",
    );
}

/// The editor protocol has to be able to say "this field points at
/// that entity" about a session both ends share. Refusing to encode a
/// live reference made an authored one undescribable.
#[test]
fn a_live_reference_round_trips() {
    let original = EntityRef::live(Entity::new(3, 1));
    let encoded = ron::to_string(&original).expect("serialises");
    let decoded: EntityRef = ron::from_str(&encoded).expect("deserialises");
    assert_eq!(decoded, original);
}

/// The two shapes have to stay tellable apart with no tag in front of
/// them, or a scene written before `Live` existed would read back as
/// the wrong variant.
#[test]
fn the_two_shapes_do_not_collide() {
    let live = ron::to_string(&EntityRef::live(Entity::new(3, 1))).expect("serialises");
    let persistent = ron::to_string(&EntityRef::same_scene(guid(3))).expect("serialises");
    assert_ne!(live, persistent);

    let decoded: EntityRef = ron::from_str(&persistent).expect("deserialises");
    assert_eq!(decoded, EntityRef::same_scene(guid(3)));
}

#[test]
fn a_persistent_reference_round_trips_through_ron() {
    let scene = Guid::new_v4();
    for original in [
        EntityRef::same_scene(guid(1)),
        EntityRef::in_scene(scene, guid(2)),
    ] {
        let encoded = ron::to_string(&original).expect("serialises");
        let decoded: EntityRef = ron::from_str(&encoded).expect("deserialises");
        assert_eq!(decoded, original);
    }
}
