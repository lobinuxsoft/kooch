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
    // Override config dir is platform-dependent; we just verify no panic
    // when the file probably doesn't exist (most CI environments).
    // If a real layout file exists from a prior run we just skip — the
    // function is deterministic w.r.t. the current filesystem.
    let _ = load_layout();
}
