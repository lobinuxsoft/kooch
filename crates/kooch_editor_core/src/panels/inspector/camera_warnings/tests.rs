//! The rig components are read off the virtual camera's entity, and nowhere else (#1342).

use super::*;
use crate::state::{ComponentDisplayInfo, ReflectedFields};
use std::any::TypeId;

fn component(name: &str) -> ComponentDisplayInfo {
    ComponentDisplayInfo {
        type_id: TypeId::of::<()>(),
        component: kooch_ecs::ComponentId(0),
        short_name: name.to_owned().into(),
        fields: ReflectedFields::Values(Vec::new()),
        field_metas: None,
        visibility: Default::default(),
    }
}

fn entity(names: &[&str]) -> Vec<EntityDisplayInfo> {
    vec![EntityDisplayInfo {
        is_prefab_instance: false,
        entity: Entity::new(0, 0),
        components: names.iter().map(|name| component(name)).collect(),
        parent: None,
        children: Vec::new(),
        depth: 0,
        global_rotation: None,
        scene: None,
        parent_global_rotation: None,
    }]
}

fn warnings(names: &[&str]) -> Vec<Orphan> {
    warnings_for(Entity::new(0, 0), &entity(names))
}

/// The case from roll-a-ball: a lookahead beside the brain, tuned for nothing.
#[test]
fn a_lookahead_off_the_vcam_is_flagged() {
    let found = warnings(&["CameraBrain", "PerspectiveCamera", "CameraLookahead"]);
    assert_eq!(found, vec![Orphan("CameraLookahead")]);
    assert!(found[0].summary().starts_with("CameraLookahead"));
}

#[test]
fn a_lookahead_on_the_vcam_is_quiet() {
    assert!(warnings(&["VirtualCamera", "CameraLookahead"]).is_empty());
}

/// A brain is not a rig, and on its own it is the normal way to author a camera.
#[test]
fn a_bare_brain_is_quiet() {
    assert!(warnings(&["CameraBrain", "PerspectiveCamera"]).is_empty());
}

#[test]
fn every_rig_component_is_checked() {
    let found = warnings(&["CameraLookahead", "CameraFraming", "CameraCollision"]);
    assert_eq!(found.len(), OF_THE_RIG.len());
}
