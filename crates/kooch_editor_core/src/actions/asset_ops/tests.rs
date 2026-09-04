use super::{to_pascal_case, to_snake_case};

#[test]
fn pascal_case_from_various_inputs() {
    assert_eq!(to_pascal_case("NewComponent"), "NewComponent");
    assert_eq!(to_pascal_case("player health"), "PlayerHealth");
    assert_eq!(to_pascal_case("enemy_ai"), "EnemyAi");
}

#[test]
fn snake_case_from_various_inputs() {
    assert_eq!(to_snake_case("NewSystem"), "new_system");
    assert_eq!(to_snake_case("PlayerHealth"), "player_health");
    assert_eq!(to_snake_case("enemy ai"), "enemy_ai");
}

/// What the scaffolds write is what the scanner reads.
///
/// 🔴 The two live apart — `templates/*.rs.tmpl` and
/// `codegen::detect` — and neither mentions the other. A comment added
/// to a template, or a tightened rule in the scan, silently produces a
/// file the editor wrote and then cannot see: the component never
/// registers, the system never runs, and there is no error anywhere
/// because both halves did exactly what they say.
#[test]
fn the_scaffolds_are_what_the_scan_detects() {
    let component = super::COMPONENT_TMPL
        .replace("{{Name}}", "Health")
        .replace("{{name}}", "health");
    let (components, _) = crate::actions::codegen::detect(&component);
    assert_eq!(
        components,
        vec!["Health".to_owned()],
        "the component scaffold is not detected as a component"
    );

    let system = super::SYSTEM_TMPL
        .replace("{{Name}}", "Movement")
        .replace("{{name}}", "movement");
    let (_, systems) = crate::actions::codegen::detect(&system);
    assert_eq!(systems.len(), 1, "the system scaffold is not detected once");
    assert_eq!(systems[0].name, "movement");
    // The scaffold carries `#[system(Update)]`, so the binding must come
    // from the attribute rather than from the fallback. They agree today;
    // this is what says so when one of them moves.
    assert_eq!(systems[0].stage, "Update");
    assert!(systems[0].gated);
}

/// The system scaffold names every stage the engine has.
///
/// 🔴 A scaffold is where an author learns what their options are, and a
/// list that is missing one is a stage nobody discovers. `Stage::ALL` is
/// the source of truth; this reads it rather than repeating it.
#[test]
fn the_scaffold_lists_every_stage() {
    let stages = include_str!("../../../../../crates/kooch_core/src/stage.rs");
    let all = stages
        .split_once("pub const ALL: [Stage; 14] = [")
        .expect("`Stage::ALL` moved or changed length")
        .1
        .split_once("];")
        .expect("`Stage::ALL` is not terminated")
        .0;
    for stage in all
        .split(',')
        .filter_map(|entry| entry.trim().strip_prefix("Stage::"))
    {
        assert!(
            super::SYSTEM_TMPL.contains(&format!("`{stage}`")),
            "the system scaffold does not mention `{stage}`, so an author reading it \
             would not know the stage exists"
        );
    }
}

// ---- a created asset has to reach the database ----------------------

use kooch_core::asset_database::AssetDatabase;
use kooch_core::resource::Resources;

use super::create_file;
use crate::actions::NewFileKind;
use crate::systems::LastScannedProject;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_create_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    dir
}

/// Resources shaped like a project mid-session: a database with things
/// in it, and a scan that has already run.
fn mid_session() -> Resources {
    let mut resources = Resources::new();
    resources.insert(AssetDatabase::default());
    resources.insert(LastScannedProject {
        root: Some(std::path::PathBuf::from("/proj")),
    });
    resources
}

/// 🔴 Writing the file is not the job. The pickers read `AssetDatabase`,
/// which lives in memory, and `save_action` already wrote the `.meta` —
/// so the identity is on disk and nothing in the editor knows it exists.
///
/// Reported as *"si creo un nuevo input action no lo reconoce como para
/// agregarlo"*. Every other asset kind goes through `write_asset`, which
/// registers and announces; this one saved and returned.
#[test]
fn a_new_input_action_is_registered() {
    let dir = scratch("input_action");
    let mut resources = mid_session();

    create_file(&mut resources, &dir, "Sprint", NewFileKind::InputAction);

    let database = resources.get::<AssetDatabase>().expect("database");
    assert!(
        database
            .path_iter()
            .any(|(p, _)| p.extension().is_some_and(|e| e == "inputaction")),
        "the action was written to disk with an identity beside it, and nothing \
         registered it — no field can reference it until the project is reopened",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 🔴 And the running project has to be told, which is the same call.
///
/// Reported separately as *"el rebuild sólo actualiza el código, no los
/// assets nuevos para que los reconozca el código"* — one cause, two
/// symptoms: `asset_saved` both registers here and announces there.
#[test]
fn a_new_input_action_asks_for_a_rescan() {
    let dir = scratch("input_action_rescan");
    let mut resources = mid_session();

    create_file(&mut resources, &dir, "Sprint", NewFileKind::InputAction);

    assert_eq!(
        resources
            .get::<LastScannedProject>()
            .and_then(|l| l.root.clone()),
        None,
        "the scan was never asked to run again, so the eager import never sees \
         the file either",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The kind beside it, which has always asked. A guard so the fix does
/// not regress the path it was copied from.
#[test]
fn a_new_build_preset_asks_too() {
    let dir = scratch("build_preset");
    let mut resources = mid_session();

    create_file(&mut resources, &dir, "Linux", NewFileKind::BuildPreset);

    assert_eq!(
        resources
            .get::<LastScannedProject>()
            .and_then(|l| l.root.clone()),
        None,
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A block tool that starts from nothing has nothing to drag, so the
/// asset is born as a cube rather than an empty mesh.
#[test]
fn a_new_block_is_a_cube() {
    let dir = scratch("block_mesh");
    let mut resources = mid_session();

    create_file(&mut resources, &dir, "Wall", NewFileKind::BlockMesh);

    let file = dir.join(format!("Wall.{}", kooch_blockmesh::BLOCK_MESH_EXTENSION));
    let text = std::fs::read_to_string(&file).expect("the block was written");
    let block: kooch_blockmesh::BlockMesh = ron::from_str(&text).expect("it parses back");
    assert_eq!(block.face_count(), 6);
    assert_eq!(block.positions().len(), 8);
}

/// Written through the same register-and-announce path every other asset
/// takes — a block nothing registered cannot be pointed at by `Block`.
#[test]
fn a_new_block_is_registered() {
    let dir = scratch("block_registered");
    let mut resources = mid_session();

    create_file(&mut resources, &dir, "Wall", NewFileKind::BlockMesh);

    let database = resources.get::<AssetDatabase>().expect("database");
    assert!(
        database.path_iter().any(|(p, _)| p
            .to_string_lossy()
            .ends_with(kooch_blockmesh::BLOCK_MESH_EXTENSION)),
        "the block was written and nothing registered it; database holds: {:?}",
        database
            .path_iter()
            .map(|(p, _)| p.display().to_string())
            .collect::<Vec<_>>(),
    );
}
