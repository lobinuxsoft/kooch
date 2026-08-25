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
