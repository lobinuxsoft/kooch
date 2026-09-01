use super::*;

fn record(name: &str, source: SystemSource) -> SystemRecord {
    SystemRecord {
        stage: Stage::Update,
        name: name.to_owned(),
        key: SystemKey::new(name),
        source,
        gpu: false,
    }
}

#[test]
fn a_catalog_splits_by_source() {
    let catalog = SystemCatalog::new(vec![
        record("kooch_render::upload", SystemSource::Engine),
        record("game::jump", SystemSource::Project),
        record("kooch_physics::step", SystemSource::Engine),
    ]);

    let project: Vec<&str> = catalog
        .from(SystemSource::Project)
        .map(|system| system.short_name())
        .collect();
    assert_eq!(project, vec!["jump"]);
    assert_eq!(catalog.from(SystemSource::Engine).count(), 2);
}

/// The panel shows one row per system; the order it shows them in is the
/// order the frame runs them, which the catalog preserves.
#[test]
fn a_catalog_keeps_its_order() {
    let catalog = SystemCatalog::new(vec![
        record("a::first", SystemSource::Engine),
        record("b::second", SystemSource::Engine),
    ]);
    let names: Vec<&str> = catalog.all().iter().map(|s| s.short_name()).collect();
    assert_eq!(names, vec!["first", "second"]);
}
