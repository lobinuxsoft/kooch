use super::*;
use crate::reflect::{EntityRef, FieldKind, Reflect, ReflectValue};

/// The link has to be a reference the scene paths recognise. As a
/// string it round-tripped as text that resolved to nothing, which is
/// why `parent_index` existed at all.
#[test]
fn the_parent_link_is_an_entity_reference() {
    let parent = Parent {
        entity: Entity::new(2, 1),
    };
    let meta = parent
        .reflect_fields()
        .iter()
        .find(|f| f.name == "entity")
        .expect("the entity field is reflected");

    assert_eq!(meta.kind, FieldKind::EntityRef);
    assert_eq!(
        parent.reflect_get("entity"),
        Some(ReflectValue::EntityRef(Some(EntityRef::live(Entity::new(
            2, 1
        ))))),
    );
}
