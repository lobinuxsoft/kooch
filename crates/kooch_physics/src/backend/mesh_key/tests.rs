use kooch_core::Guid;
use kooch_ecs::entity::Entity;

use super::MeshKey;

#[test]
fn an_asset_and_an_entity_never_collide() {
    // 🔴 The two spaces are separate. An entity index that happens to
    // match nothing about a GUID must not be able to read its mesh.
    let guid = Guid::new_v4();
    assert_ne!(MeshKey::Asset(guid), MeshKey::Owned(Entity::new(0, 0)));
}

#[test]
fn two_entities_are_two_keys() {
    // Two blocks from one source are two shapes the moment either is
    // scaled; sharing one entry is right only by luck.
    assert_ne!(
        MeshKey::Owned(Entity::new(1, 0)),
        MeshKey::Owned(Entity::new(2, 0)),
    );
}

#[test]
fn the_same_asset_is_the_same_key() {
    let guid = Guid::new_v4();
    assert_eq!(MeshKey::Asset(guid), MeshKey::Asset(guid));
}
