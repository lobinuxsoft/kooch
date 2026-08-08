use super::*;

#[derive(Debug, PartialEq)]
struct Mesh(u32);

#[derive(Debug, PartialEq)]
struct Texture(&'static str);

#[test]
fn insert_then_get_round_trip() {
    let mut assets = Assets::<Mesh>::new();
    let handle = assets.insert(Mesh(42));
    assert_eq!(assets.get(handle), Some(&Mesh(42)));
}

#[test]
fn distinct_inserts_yield_distinct_handles() {
    let mut assets = Assets::<Mesh>::new();
    let h1 = assets.insert(Mesh(1));
    let h2 = assets.insert(Mesh(2));
    assert_ne!(h1, h2);
    assert_eq!(assets.get(h1), Some(&Mesh(1)));
    assert_eq!(assets.get(h2), Some(&Mesh(2)));
}

#[test]
fn remove_returns_asset_and_invalidates_handle() {
    let mut assets = Assets::<Mesh>::new();
    let handle = assets.insert(Mesh(7));
    assert_eq!(assets.remove(handle), Some(Mesh(7)));
    assert_eq!(assets.get(handle), None);
    assert_eq!(assets.remove(handle), None);
}

#[test]
fn stale_handle_after_slot_reuse_returns_none() {
    let mut assets = Assets::<Mesh>::new();
    let stale = assets.insert(Mesh(100));
    assets.remove(stale);
    // New insert may reuse the freed slot index, but generation
    // bumps so the stale handle no longer matches.
    let _fresh = assets.insert(Mesh(200));
    assert_eq!(assets.get(stale), None);
}

#[test]
fn contains_tracks_live_state() {
    let mut assets = Assets::<Mesh>::new();
    let h = assets.insert(Mesh(0));
    assert!(assets.contains(h));
    assets.remove(h);
    assert!(!assets.contains(h));
}

#[test]
fn len_and_is_empty() {
    let mut assets = Assets::<Mesh>::new();
    assert!(assets.is_empty());
    assert_eq!(assets.len(), 0);

    let h1 = assets.insert(Mesh(1));
    let h2 = assets.insert(Mesh(2));
    assert_eq!(assets.len(), 2);
    assert!(!assets.is_empty());

    assets.remove(h1);
    assert_eq!(assets.len(), 1);

    assets.remove(h2);
    assert!(assets.is_empty());
}

#[test]
fn iter_visits_all_assets() {
    let mut assets = Assets::<Mesh>::new();
    let _h1 = assets.insert(Mesh(10));
    let _h2 = assets.insert(Mesh(20));
    let _h3 = assets.insert(Mesh(30));

    let mut values: Vec<u32> = assets.iter().map(|(_, m)| m.0).collect();
    values.sort();
    assert_eq!(values, vec![10, 20, 30]);
}

#[test]
fn iter_mut_allows_in_place_mutation() {
    let mut assets = Assets::<Mesh>::new();
    let h = assets.insert(Mesh(1));
    for (_, mesh) in assets.iter_mut() {
        mesh.0 *= 10;
    }
    assert_eq!(assets.get(h), Some(&Mesh(10)));
}

#[test]
fn clear_drops_everything() {
    let mut assets = Assets::<Mesh>::new();
    let h1 = assets.insert(Mesh(1));
    let h2 = assets.insert(Mesh(2));
    assets.clear();
    assert_eq!(assets.len(), 0);
    assert_eq!(assets.get(h1), None);
    assert_eq!(assets.get(h2), None);
}

#[test]
fn handle_is_copy_and_clone() {
    let mut assets = Assets::<Mesh>::new();
    let h = assets.insert(Mesh(1));
    let h2 = h;
    let h3 = h;
    assert_eq!(h2, h3);
    assert_eq!(assets.get(h2), Some(&Mesh(1)));
}

#[test]
fn handles_for_different_asset_types_are_independent() {
    let mut meshes = Assets::<Mesh>::new();
    let mut textures = Assets::<Texture>::new();

    let mesh_h = meshes.insert(Mesh(1));
    let tex_h = textures.insert(Texture("albedo"));

    assert_eq!(meshes.get(mesh_h), Some(&Mesh(1)));
    assert_eq!(textures.get(tex_h), Some(&Texture("albedo")));
}

#[test]
fn debug_format_includes_type_name() {
    let mut assets = Assets::<Mesh>::new();
    let h = assets.insert(Mesh(0));
    let dbg = format!("{h:?}");
    assert!(dbg.contains("Mesh"));
    assert!(dbg.contains("Handle"));
}
