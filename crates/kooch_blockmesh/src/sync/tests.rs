use kooch_core::resource::Resources;
use kooch_ecs::Entity;

use super::{BuiltBlocks, revision_of, sync_blocks};

use crate::BlockMesh;

fn any_entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

#[test]
fn nothing_is_built_at_first() {
    assert!(!BuiltBlocks::default().is_built(any_entity(1), 0));
}

#[test]
fn forgetting_asks_for_a_rebuild() {
    let entity = any_entity(1);
    let mut built = BuiltBlocks::default();
    built.built.insert(entity, 7);
    assert!(built.is_built(entity, 7));
    built.forget(entity);
    assert!(!built.is_built(entity, 7));
}

#[test]
fn forgetting_an_unknown_block_is_quiet() {
    BuiltBlocks::default().forget(any_entity(9));
}

#[test]
fn a_bare_world_syncs_nothing() {
    // No registry, no assets, no caches — the editor's first frame.
    let mut resources = Resources::new();
    sync_blocks(&mut resources);
}

/// 🔴 An edit changes the revision, so a block built at one is not built at the next. This is the
/// whole mechanism now that the geometry is on the component: there is no file revision to read.
#[test]
fn an_edited_block_is_not_built() {
    let entity = any_entity(1);
    let mut built = BuiltBlocks::default();
    let before = revision_of(&BlockMesh::cuboid(glam::Vec3::ONE));
    built.built.insert(entity, before);

    assert!(built.is_built(entity, before));
    let after = revision_of(&BlockMesh::cuboid(glam::Vec3::splat(2.0)));
    assert!(built.is_built(entity, after) == false, "the geometry moved");
}

/// Two meshes whose corner lists read the same but whose faces are split differently are two
/// different meshes. The guard against a future revision that flattens the corners and loses the
/// split — which would leave the second one never rebuilt.
#[test]
fn a_resplit_face_changes_the_revision() {
    let mut one = BlockMesh::default();
    one.positions = vec![glam::Vec3::ZERO; 4];
    one.face_corners = vec![0, 1, 2, 3];
    one.face_starts = vec![0, 4];

    let mut split = one.clone();
    split.face_starts = vec![0, 2, 4];

    assert_ne!(revision_of(&one), revision_of(&split));
}
