use kooch_core::Guid;
use kooch_core::resource::Resources;

use super::{BuiltBlocks, sync_blocks};
use kooch_core::assets::Assets;

use crate::BlockMesh;

/// A handle to nothing in particular — these tests are about which
/// GUIDs are remembered, not what they resolve to.
fn any_handle() -> kooch_core::assets::Handle<BlockMesh> {
    Assets::<BlockMesh>::new().insert(BlockMesh::default())
}

/// A source built from the bytes it had at revision zero.
fn built_now() -> super::Built {
    super::Built {
        handle: any_handle(),
        revision: 0,
    }
}

#[test]
fn nothing_is_built_at_first() {
    assert!(!BuiltBlocks::default().is_built(Guid::new_v4(), 0));
}

#[test]
fn forgetting_asks_for_a_rebuild() {
    let guid = Guid::new_v4();
    let mut built = BuiltBlocks::default();
    built.built.insert(guid, built_now());
    assert!(built.is_built(guid, 0));
    built.forget(guid);
    assert!(!built.is_built(guid, 0));
}

#[test]
fn forgetting_an_unknown_source_is_quiet() {
    BuiltBlocks::default().forget(Guid::new_v4());
}

#[test]
fn a_bare_world_syncs_nothing() {
    // No registry, no assets, no caches — the editor's first frame.
    let mut resources = Resources::new();
    sync_blocks(&mut resources);
}

/// 🔴 A reload overwrites the value under the same handle, so a source built at one revision is not
/// built at the next.
#[test]
fn a_rewritten_source_is_not_built() {
    let guid = Guid::new_v4();
    let mut built = BuiltBlocks::default();
    built.built.insert(guid, built_now());

    assert!(built.is_built(guid, 0));
    assert!(!built.is_built(guid, 1), "the file changed under it");
}
