use super::stale_source;

/// 🔴 A component written and not yet compiled is simply absent from
/// the add-component menu, and nothing says why: the editor loads
/// this library, it does not build it.
///
/// Reported from a real session — the `.so` was 21 minutes older than
/// the component that "did not exist", and the time went into the
/// derive, `registrations.rs` and the `#[reflect]` attribute, none of
/// which were wrong.
#[test]
fn a_source_newer_than_the_library_is_reported() {
    let dir = std::env::temp_dir().join("kooch_stale_plugin_test");
    let src = dir.join("src").join("components");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&src).expect("temp dirs");

    let library = dir.join("libproject.so");
    std::fs::write(&library, b"not really a library").expect("write library");

    // Written after, which is exactly the reported situation.
    let component = src.join("input.rs");
    std::fs::write(&component, b"pub struct PlayerInput;").expect("write source");
    filetime_bump(&component);

    assert_eq!(
        stale_source(&dir, &library).as_deref(),
        Some(component.as_path()),
        "a source newer than the library went unreported"
    );

    // And the other way round: a fresh build is quiet, or the warning
    // fires on every open and stops meaning anything.
    let rebuilt = dir.join("librebuilt.so");
    std::fs::write(&rebuilt, b"newer").expect("write");
    filetime_bump(&rebuilt);
    assert_eq!(
        stale_source(&dir, &rebuilt),
        None,
        "a library newer than every source was reported as stale"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// No sources at all is not a complaint: a project that defines no
/// components is a normal project.
#[test]
fn a_project_with_no_sources_is_quiet() {
    let dir = std::env::temp_dir().join("kooch_stale_plugin_empty");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let library = dir.join("libproject.so");
    std::fs::write(&library, b"x").expect("write");

    assert_eq!(stale_source(&dir, &library), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Rewrites `path` until its mtime is strictly later than it was, so
/// the test does not depend on filesystem timestamp granularity —
/// which on some mounts is a whole second.
fn filetime_bump(path: &std::path::Path) {
    let before = path.metadata().and_then(|m| m.modified()).expect("mtime");
    for _ in 0..100 {
        std::fs::write(path, b"touched").expect("rewrite");
        let now = path.metadata().and_then(|m| m.modified()).expect("mtime");
        if now > before {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("could not make {} newer", path.display());
}
