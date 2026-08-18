use super::*;

fn ctx_free_load(bytes: &[u8]) -> AssetResult<SceneDocument> {
    // The context only carries the source path, which this loader does
    // not consult — a prefab's contents are self-describing.
    let mut ctx = LoadContext::new(std::path::Path::new("x.prefab"));
    PrefabLoader.load(bytes, &mut ctx)
}

/// The extension is the one that names the single-root invariant, not
/// the scene one — registering both against the same loader would make
/// every scene show up in a prefab picker.
#[test]
fn the_loader_claims_prefabs_and_not_scenes() {
    let extensions = PrefabLoader.extensions();
    assert!(extensions.contains(&kooch_core::scene_paths::PREFAB_EXTENSION));
    assert!(!extensions.contains(&kooch_core::scene_paths::SCENE_EXTENSION));
}

#[test]
fn a_prefab_round_trips_through_the_loader() {
    let document = SceneDocument {
        id: Guid::new_v4(),
        name: "Ball".into(),
        version: "0.1.0".into(),
        entities: Vec::new(),
    };
    let text = ron::ser::to_string(&document).unwrap();
    let loaded = ctx_free_load(text.as_bytes()).expect("its own output should parse");
    assert_eq!(loaded.name, "Ball");
    assert_eq!(loaded.id, document.id);
}

/// A truncated or hand-edited file has to fail as an error rather than
/// panic: it arrives from disk, so it is input, not a bug.
#[test]
fn a_malformed_prefab_is_an_error() {
    assert!(ctx_free_load(b"(id: ").is_err());
    assert!(ctx_free_load(&[0xff, 0xfe]).is_err(), "invalid UTF-8");
}
