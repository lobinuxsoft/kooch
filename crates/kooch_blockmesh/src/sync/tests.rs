use kooch_core::Guid;
use kooch_core::resource::Resources;

use super::{BuiltBlocks, sync_blocks};

#[test]
fn nothing_is_built_at_first() {
    assert!(!BuiltBlocks::default().is_built(Guid::new_v4()));
}

#[test]
fn forgetting_asks_for_a_rebuild() {
    let guid = Guid::new_v4();
    let mut built = BuiltBlocks::default();
    built.built.insert(guid);
    assert!(built.is_built(guid));
    built.forget(guid);
    assert!(!built.is_built(guid));
}

#[test]
fn forget_all_clears_every_source() {
    let (first, second) = (Guid::new_v4(), Guid::new_v4());
    let mut built = BuiltBlocks::default();
    built.built.insert(first);
    built.built.insert(second);
    built.forget_all();
    assert!(!built.is_built(first));
    assert!(!built.is_built(second));
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
