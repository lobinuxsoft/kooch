use std::path::PathBuf;

use kooch_core::Guid;

use super::*;

fn guid(n: u8) -> Guid {
    let mut bytes = [0u8; 16];
    bytes[0] = n;
    Guid::from_bytes(bytes)
}

/// The two panels that show the scene reach the scene's history,
/// whatever happens to be selected in the Asset Browser.
#[test]
fn the_scene_panels_reach_the_scene() {
    let asset = Some((guid(7), AssetKind::Asset));
    assert_eq!(
        resolve(Some(EditorTab::World), asset, None),
        Some(Document::World),
    );
    assert_eq!(
        resolve(Some(EditorTab::View), asset, None),
        Some(Document::World),
    );
}

/// 🔴 The Inspector is not a document: its history is whatever it is
/// showing. This is the whole reason the routing exists — the same panel
/// edits three different things and a Ctrl+Z has to follow.
#[test]
fn the_inspector_follows_its_subject() {
    assert_eq!(
        resolve(
            Some(EditorTab::Inspector),
            Some((guid(1), AssetKind::Prefab)),
            None
        ),
        Some(Document::Prefab(guid(1))),
    );
    assert_eq!(
        resolve(
            Some(EditorTab::Inspector),
            Some((guid(2), AssetKind::Asset)),
            None
        ),
        Some(Document::Asset(guid(2))),
    );
    // Nothing selected means an entity is: the Inspector shows the world.
    assert_eq!(
        resolve(Some(EditorTab::Inspector), None, None),
        Some(Document::World),
    );
}

/// The map it has open, and nothing when it has none — a panel with no
/// document has no history to reach.
#[test]
fn the_input_map_needs_a_map() {
    let path = PathBuf::from("/p/assets/player.inputaction");
    assert_eq!(
        resolve(Some(EditorTab::InputMap), None, Some(&path)),
        Some(Document::InputMap(path.clone())),
    );
    assert_eq!(resolve(Some(EditorTab::InputMap), None, None), None);
}

/// The panels whose edits are files, or aren't edits. Deliberately
/// `None` rather than falling back to the scene: a Ctrl+Z in the Console
/// undoing something in the viewport is exactly the surprise this
/// routing exists to prevent.
#[test]
fn the_other_panels_reach_nothing() {
    for tab in [
        EditorTab::Console,
        EditorTab::AssetBrowser,
        EditorTab::Archetypes,
        EditorTab::Build,
    ] {
        assert_eq!(resolve(Some(tab), None, None), None, "{tab:?}");
    }
    assert_eq!(resolve(None, None, None), None);
}

/// Two prefabs are two histories. Keyed by guid, so the entry in one
/// cannot be undone from the other.
#[test]
fn each_prefab_is_its_own() {
    assert_ne!(Document::Prefab(guid(1)), Document::Prefab(guid(2)));
    assert_eq!(Document::Prefab(guid(1)), Document::Prefab(guid(1)));
}
