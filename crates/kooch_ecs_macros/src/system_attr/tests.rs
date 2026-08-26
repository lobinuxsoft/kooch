use super::STAGES;

/// The stage list matches the engine's, name for name and in order.
///
/// 🔴 A proc-macro crate cannot depend on `kooch_core`, so this list
/// is a copy — and a copy that drifts is a new stage nobody can bind
/// to, reported as "not a stage" while it plainly is one. Reading the
/// enum's source is ugly and it is the only thing that actually
/// catches the drift.
#[test]
fn the_stages_match_the_engine() {
    let source = include_str!("../../../kooch_core/src/stage.rs");
    let all = source
        .split_once("pub const ALL: [Stage; 14] = [")
        .expect("`Stage::ALL` moved or changed length")
        .1
        .split_once("];")
        .expect("`Stage::ALL` is not terminated")
        .0;
    let engine: Vec<&str> = all
        .split(',')
        .filter_map(|entry| entry.trim().strip_prefix("Stage::"))
        .collect();
    assert_eq!(
        engine,
        STAGES.to_vec(),
        "the attribute's stage list drifted from `Stage::ALL`"
    );
}
