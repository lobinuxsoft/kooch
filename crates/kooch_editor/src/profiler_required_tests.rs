//! The editor ships with its profiler, and this is what says so.

/// 🔴 An editor without a profiler is not an editor.
///
/// It is the tool that answers "why is this frame slow" — the whole
/// reason #785 exists, and what every performance session of this engine
/// has run on. A build of it that cannot answer that looks completely
/// normal until somebody opens the panel and reads
/// *"Profiling is not compiled into this build"*, which has now happened
/// twice: once mid-session, and once in a smoke where the editor had been
/// rebuilt with a plain `cargo build -p kooch_editor`.
///
/// The feature is `default` for that reason. This fails if it is ever
/// removed from `default`, or if someone builds with
/// `--no-default-features` and expects a usable editor.
///
/// ⚠️ Not the same decision as the engine's `profiling` feature, which is
/// opt-in and must stay that way: a shipped **game** carries no
/// instrumentation (#558). The editor is a tool.
#[test]
fn the_editor_ships_with_its_profiler() {
    assert!(
        cfg!(feature = "profiling"),
        "the editor was built without its profiler — check `default` in \
         crates/kooch_editor/Cargo.toml, and that nothing passed \
         --no-default-features",
    );
}
