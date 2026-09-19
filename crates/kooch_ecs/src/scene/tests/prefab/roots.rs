//! root_index: a document's one root, or none.

use super::*;

// -- root_index ---------------------------------------------------------

pub(super) fn described(name: &str, components: Vec<ComponentDescription>) -> EntityDescription {
    EntityDescription {
        name: name.into(),
        parent_index: None,
        parent: None,
        components,
    }
}

pub(super) fn parented_to(id: u64) -> ComponentDescription {
    ComponentDescription {
        type_name: "kooch_ecs::hierarchy::Parent".into(),
        fields: vec![(
            "entity".into(),
            ReflectValue::EntityRef(Some(EntityRef::Persistent {
                scene: None,
                id: EntityGuid::new(id).unwrap(),
            })),
        )],
    }
}

pub(super) fn identified(id: u64) -> ComponentDescription {
    ComponentDescription {
        type_name: "kooch_ecs::persistent_id::PersistentId".into(),
        fields: vec![("id".into(), ReflectValue::U64(id))],
    }
}

pub(super) fn document(entities: Vec<EntityDescription>) -> SceneDocument {
    SceneDocument {
        id: Guid::new_v4(),
        name: "Prefab".into(),
        version: "0.1.0".into(),
        entities,
    }
}

#[test]
fn one_tree_has_one_root() {
    let doc = document(vec![
        described("Root", vec![identified(1)]),
        described("Child", vec![parented_to(1)]),
    ]);
    assert_eq!(doc.root_index().unwrap(), 0);
}

/// Instancing as a unit needs one entity to place and parent. Several
/// roots have no such entity, and silently picking the first would attach
/// the rest to the scene root where nothing would ever move them.
#[test]
fn several_roots_cannot_be_instanced_as_a_unit() {
    let doc = document(vec![
        described("A", vec![identified(1)]),
        described("B", vec![identified(2)]),
        described("C", vec![parented_to(1)]),
    ]);
    assert!(matches!(
        doc.root_index(),
        Err(SceneError::NotASingleRoot { roots: 2 })
    ));
}

#[test]
fn an_empty_document_has_no_root() {
    assert!(matches!(
        document(vec![]).root_index(),
        Err(SceneError::NotASingleRoot { roots: 0 })
    ));
}
