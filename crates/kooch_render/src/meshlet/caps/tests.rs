use super::*;

#[test]
fn default_is_conservative() {
    let caps = MeshletDebugCaps::default();
    assert!(!caps.supports_texture_atomic());
}

#[test]
fn from_flags_round_trips() {
    assert!(MeshletDebugCaps::from_flags(true).supports_texture_atomic());
    assert!(!MeshletDebugCaps::from_flags(false).supports_texture_atomic());
}
