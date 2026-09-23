use super::*;

const EVERY: [Color32; 11] = [
    family::PREFAB,
    family::SCENE,
    family::MESH,
    family::MATERIAL,
    family::SHADER,
    family::TEXTURE,
    family::AUDIO,
    family::INPUT,
    family::BLOCK,
    state::MAIN_SCENE,
    state::DIRTY,
];

/// A colour that means two things means nothing: every one has to be told from every other.
#[test]
fn every_colour_is_its_own() {
    for (at, colour) in EVERY.iter().enumerate() {
        for other in &EVERY[at + 1..] {
            let far = (colour.r() as i32 - other.r() as i32).abs()
                + (colour.g() as i32 - other.g() as i32).abs()
                + (colour.b() as i32 - other.b() as i32).abs();
            assert!(
                far > 60,
                "{colour:?} and {other:?} are the same colour to a reader"
            );
        }
    }
}

/// Every colour has to be readable on the editor's dark panel, which is around 27/27/27.
#[test]
fn every_colour_reads_on_the_panel() {
    for colour in EVERY {
        let luma =
            0.2126 * colour.r() as f32 + 0.7152 * colour.g() as f32 + 0.0722 * colour.b() as f32;
        assert!(luma > 120.0, "{colour:?} is too dark to read at {luma}");
    }
}

#[test]
fn a_file_takes_its_family_from_its_extension() {
    assert_eq!(of_extension("Player.prefab"), Some(family::PREFAB));
    assert_eq!(of_extension("level.scene"), Some(family::SCENE));
    assert_eq!(of_extension("rock.glb"), Some(family::MESH));
    assert_eq!(
        of_extension("notes.txt"),
        None,
        "an unknown file keeps the plain text colour"
    );
}

#[test]
fn a_typed_asset_takes_its_family_from_its_type() {
    assert_eq!(
        of_type("kooch_render::material::asset::Material"),
        Some(family::MATERIAL),
    );
    assert_eq!(
        of_type("kooch_render::meshlet::asset::MeshletMesh"),
        Some(family::MESH),
    );
}
