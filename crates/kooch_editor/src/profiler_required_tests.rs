//! The editor ships with its profiler, and this is what says so.

/// 🔴 The editor's profiler feature is `default` (#785): a plain `cargo build -p kooch_editor`
/// shipped a panel saying profiling was not compiled in, twice. The engine's `profiling` stays
/// opt-in — games carry no instrumentation (#558).
#[test]
fn the_editor_ships_with_its_profiler() {
    assert!(
        cfg!(feature = "profiling"),
        "the editor was built without its profiler — check `default` in \
         crates/kooch_editor/Cargo.toml, and that nothing passed \
         --no-default-features",
    );
}
