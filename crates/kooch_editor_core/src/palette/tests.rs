use super::*;

const FAMILIES: [Family; 12] = [
    Family::Prefab,
    Family::Scene,
    Family::Mesh,
    Family::Material,
    Family::Shader,
    Family::Texture,
    Family::Audio,
    Family::Input,
    Family::Block,
    Family::Settings,
    Family::Code,
    Family::Notes,
];

/// Every colour a row can be drawn in, families and states alike.
fn every_colour() -> Vec<Color32> {
    let mut colours: Vec<Color32> = FAMILIES.iter().filter_map(|f| f.colour()).collect();
    colours.push(state::MAIN_SCENE);
    colours.push(state::DIRTY);
    colours
}

/// A colour that means two things means nothing: every one has to be told from every other.
#[test]
fn every_colour_is_its_own() {
    let colours = every_colour();
    for (at, colour) in colours.iter().enumerate() {
        for other in &colours[at + 1..] {
            let far = (colour.r() as i32 - other.r() as i32).abs()
                + (colour.g() as i32 - other.g() as i32).abs()
                + (colour.b() as i32 - other.b() as i32).abs();
            assert!(
                far > 60,
                "{colour:?} and {other:?} are the same colour to a reader",
            );
        }
    }
}

/// Every colour has to be readable on the editor's dark panel.
#[test]
fn every_colour_reads_on_the_panel() {
    for colour in every_colour() {
        let luma =
            0.2126 * colour.r() as f32 + 0.7152 * colour.g() as f32 + 0.0722 * colour.b() as f32;
        assert!(luma > 120.0, "{colour:?} is too dark to read at {luma}");
    }
}

/// Icon and colour come from one table, so a row cannot say two things.
#[test]
fn every_family_has_its_own_icon() {
    for (at, family) in FAMILIES.iter().enumerate() {
        for other in &FAMILIES[at + 1..] {
            assert_ne!(
                family.icon(),
                other.icon(),
                "{family:?} and {other:?} draw the same icon",
            );
        }
    }
    // The nine asset families carry a colour; settings, code and notes stay plain.
    assert_eq!(FAMILIES.iter().filter(|f| f.colour().is_some()).count(), 9);
}

#[test]
fn a_file_takes_its_family_from_its_extension() {
    assert_eq!(of_extension("Player.prefab"), Some(Family::Prefab));
    assert_eq!(of_extension("level.scene"), Some(Family::Scene));
    assert_eq!(of_extension("rock.glb"), Some(Family::Mesh));
    assert_eq!(
        of_extension("mystery.zzz"),
        None,
        "an unknown file is plain"
    );
}

/// 🔴 The bug: a typed asset fell into `of_type`, and a type that table does not name lost what its
/// extension knew — prefabs, blocks and textures all came out plain.
#[test]
fn a_typed_asset_still_reads_its_extension() {
    assert_eq!(
        of_asset("Player.prefab", Some("kooch_ecs::prefab::asset::Prefab")),
        Some(Family::Prefab),
    );
    assert_eq!(
        of_asset("Arch.block", Some("kooch_blockmesh::asset::BlockMesh")),
        Some(Family::Block),
    );
    assert_eq!(
        of_asset("dark_texture_01.png", Some("kooch_render::image::Image")),
        Some(Family::Texture),
    );
    assert_eq!(of_asset("Jump.inputaction", None), Some(Family::Input));
}

#[test]
fn a_typed_asset_takes_its_family_from_its_type() {
    assert_eq!(
        of_type("kooch_render::material::asset::Material"),
        Some(Family::Material),
    );
    assert_eq!(
        of_type("kooch_render::meshlet::asset::MeshletMesh"),
        Some(Family::Mesh),
    );
}

/// Settings, code and notes are not assets: they keep the plain row.
#[test]
fn settings_and_code_stay_plain() {
    for name in ["project.kooch", "Cargo.toml", "main.rs", "README.md"] {
        let family = of_asset(name, None).expect("these are named");
        assert_eq!(family.colour(), None, "{name} was coloured");
    }
}
