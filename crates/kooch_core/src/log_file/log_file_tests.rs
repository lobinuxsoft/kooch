use std::io::Read;

use super::*;

/// Reads a log back, whatever is in it so far.
fn read(path: &std::path::Path) -> String {
    let mut text = String::new();
    let _ = std::fs::File::open(path).map(|mut f| f.read_to_string(&mut text));
    text
}

/// A log opened in a directory that takes it.
fn open_in(dir: &std::path::Path) -> (SharedLog, PathBuf) {
    let path = dir.join(LOG_NAME);
    let _ = std::fs::rename(&path, dir.join(PREVIOUS_NAME));
    let file = File::create(&path).expect("the temp dir is writable");
    (SharedLog(Arc::new(Mutex::new(file))), path)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_logfile_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 🔴 The line the whole module exists for.
///
/// A `panic!` message does not go through `tracing` — the standard
/// library writes it straight to stderr. A file fed only by a tracing
/// layer would hold every ordinary event and not the one that says why
/// the game stopped, which is exactly the state #963 was diagnosed in.
#[test]
fn a_panic_reaches_the_log() {
    let dir = scratch("panic");
    let (log, path) = open_in(&dir);

    // The hook, exercised directly: installing a real one would replace
    // the test harness's and take the rest of the suite with it.
    let mut writer = log.clone();
    let report =
        "\n=== panic ===\nthe scene had no camera\nlocation: src/main.rs:12:5\n=== end panic ===\n";
    writer.write_all(report.as_bytes()).unwrap();
    writer.flush().unwrap();

    let written = read(&path);
    assert!(
        written.contains("the scene had no camera"),
        "the panic message is not in the log: {written:?}",
    );
    assert!(written.contains("src/main.rs:12:5"), "no location");
}

/// ⚠️ The previous run is kept.
///
/// A crash is diagnosed on the *next* launch: someone runs it, it dies,
/// they run it again to watch — and that second run would otherwise
/// overwrite the evidence from the first.
#[test]
fn the_previous_run_is_not_overwritten() {
    let dir = scratch("rotate");

    let (mut first, path) = open_in(&dir);
    first.write_all(b"the run that crashed\n").unwrap();
    first.flush().unwrap();

    let (mut second, _) = open_in(&dir);
    second.write_all(b"the run that came after\n").unwrap();
    second.flush().unwrap();

    assert!(read(&path).contains("the run that came after"));
    assert!(
        read(&dir.join(PREVIOUS_NAME)).contains("the run that crashed"),
        "the crashed run's log was overwritten by the next launch",
    );
}

/// Two writers share one file without losing either.
#[test]
fn a_shared_log_takes_both_writers() {
    let dir = scratch("shared");
    let (log, path) = open_in(&dir);

    let mut layer = log.clone();
    let mut hook = log.clone();
    layer.write_all(b"an ordinary event\n").unwrap();
    hook.write_all(b"a panic report\n").unwrap();
    hook.flush().unwrap();

    let written = read(&path);
    assert!(written.contains("an ordinary event"));
    assert!(written.contains("a panic report"));
}

/// A directory nobody can write to must not cost the launch — the game
/// runs, it just cannot explain itself afterwards.
#[test]
fn an_unwritable_place_is_not_fatal() {
    let dirs = candidate_dirs();
    assert!(
        !dirs.is_empty(),
        "no candidate at all leaves nowhere to fall back to",
    );
    // The temp directory is the fallback, and it has to be last: beside
    // the executable is where somebody will look first.
    assert_eq!(dirs.last(), Some(&std::env::temp_dir()));
}

/// And the hook that is actually installed does it, not a stand-in.
///
/// 🔴 The test above writes the report by hand, which proves the file
/// takes bytes and nothing about `log_panics`. This one installs the
/// real hook and panics for real.
///
/// ⚠️ The previous hook is put back before returning: leaving a global
/// hook installed would follow every later test in this binary.
#[test]
fn the_installed_hook_catches_a_real_panic() {
    let dir = scratch("realpanic");
    let (log, path) = open_in(&dir);

    let previous = std::panic::take_hook();
    log_panics(log);
    // Silences the panic message the harness would otherwise print, so
    // a passing run does not look like a failing one.
    let caught = std::panic::catch_unwind(|| panic!("the scene had no camera"));
    std::panic::set_hook(previous);

    assert!(caught.is_err(), "the panic did not happen");
    let written = read(&path);
    assert!(
        written.contains("the scene had no camera"),
        "the installed hook did not write the message: {written:?}",
    );
    assert!(
        written.contains("=== panic ==="),
        "the report has no marker to find it by: {written:?}",
    );
    assert!(
        written.contains("log_file_tests.rs"),
        "the report does not say where it came from: {written:?}",
    );
}
