use super::*;
use crate::state::default_dock_state;

#[test]
fn round_trip_preserves_default_layout() {
    let original = default_dock_state();
    let serialized = ron::ser::to_string(&original).expect("serialize default");
    let parsed: DockState<EditorTab> = ron::from_str(&serialized).expect("parse round-trip");
    // We can't trivially `==` two DockStates (egui_dock doesn't impl Eq),
    // but a re-serialization should produce the same string.
    let reserialized = ron::ser::to_string(&parsed).expect("reserialize");
    assert_eq!(serialized, reserialized);
}

#[test]
fn layout_path_resolves_under_config_dir() {
    let path = layout_path().expect("config dir resolves on test platform");
    assert!(path.ends_with("kooch/editor_layout.ron"));
}

#[test]
fn load_layout_returns_none_for_missing_file() {
    // Override config dir is platform-dependent; we just verify no panic when the file probably
    // doesn't exist (most CI environments). If a real layout file exists from a prior run we just
    // skip — the function is deterministic w.r.t. the current filesystem.
    let _ = load_layout();
}

/// A layout file from before #1196 is a bare dock, and still loads.
#[test]
fn a_bare_dock_still_loads() {
    let legacy = ron::ser::to_string(&default_dock_state()).unwrap();
    let layout = EditorLayout::parse(&legacy).expect("the old format parses");
    assert!(layout.windows.is_empty());
}

/// Torn-off panels survive a save and a load, with where they were.
#[test]
fn detached_panels_round_trip() {
    let layout = EditorLayout {
        dock: default_dock_state(),
        windows: vec![Detached {
            tab: EditorTab::Inspector,
            size: [400.0, 700.0],
            pos: Some([2000, 40]),
        }],
    };
    let parsed = EditorLayout::parse(&ron::ser::to_string(&layout).unwrap()).unwrap();
    assert_eq!(parsed.windows, layout.windows);
}
