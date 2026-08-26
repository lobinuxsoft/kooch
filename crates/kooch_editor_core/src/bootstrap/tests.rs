/// 🔴 `install_own_engine` reads the root that `set_engine_root`
/// resolves, and systems in a stage run in the order they were added.
/// Swap the two lines and the editor silently installs nothing — which
/// is the bug this pair exists to fix, back again and just as invisible.
///
/// Pinned against the source because there is no editor to boot in a
/// test, the same reason `plugin/tests.rs` pins the frame loop's order
/// that way.
#[test]
fn the_root_is_resolved_first() {
    let source = include_str!("../bootstrap.rs");
    let root = source
        .find("Stage::Startup, set_engine_root")
        .expect("set_engine_root is no longer registered at startup");
    let install = source
        .find("Stage::Startup, install_own_engine")
        .expect("install_own_engine is no longer registered at startup");
    assert!(
        root < install,
        "install_own_engine now runs before set_engine_root, so it has no engine root to \
         install from and writes nothing",
    );
}
