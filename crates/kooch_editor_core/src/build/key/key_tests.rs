use super::*;

/// 🔴 `KEY_ENV` is the **process's** environment, and cargo runs tests on
/// several threads. Two of these set it; every other one calls
/// `project_key`, which reads it — so without a lock, a test that expects
/// a key on disk intermittently gets the one a sibling had just exported
/// and finds nothing written. Every test here takes it, including the
/// ones that never touch the variable, because they are the ones that
/// lose the race.
static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The lock, surviving a sibling's panic: a poisoned mutex would turn one
/// failure into every failure and hide which test broke.
fn alone() -> std::sync::MutexGuard<'static, ()> {
    ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_projkey_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 🔴 Generated once and kept. Regenerated per build, the editor could
/// not open yesterday's pack to check it, and anyone holding an older
/// release would be holding a file nobody can read.
#[test]
fn the_key_is_generated_once_and_kept() {
    let _alone = alone();
    let dir = tmp("stable");
    assert!(!has_key(&dir));

    let first = project_key(&dir).unwrap();
    assert!(has_key(&dir));
    let second = project_key(&dir).unwrap();

    assert_eq!(first, second);
}

/// Breaking one project must say nothing about the next.
#[test]
fn two_projects_get_two_keys() {
    let _alone = alone();
    let (a, b) = (tmp("proj_a"), tmp("proj_b"));

    assert_ne!(project_key(&a).unwrap(), project_key(&b).unwrap());
}

/// It goes under `.kooch/`, which the scaffold's `.gitignore` excludes —
/// a repository carrying a key has published it, and history keeps it
/// published after the file is deleted.
#[test]
fn the_key_lands_where_git_ignores_it() {
    let _alone = alone();
    let dir = tmp("location");
    project_key(&dir).unwrap();

    assert_eq!(key_path(&dir), dir.join(".kooch").join("pack.key"));
    assert!(key_path(&dir).is_file());
}

/// What CI uses: the key arrives from a secret store, and nothing is
/// written to the checkout.
#[test]
fn the_environment_wins_and_writes_nothing() {
    let _alone = alone();
    let dir = tmp("env");
    let wanted = kooch_pack::PackKey::generate();
    // SAFETY: `alone()` is held, so no sibling test is reading the
    // environment while this one changes it.
    unsafe { std::env::set_var(KEY_ENV, wanted.to_hex()) };

    let got = project_key(&dir).unwrap();

    unsafe { std::env::remove_var(KEY_ENV) };
    assert_eq!(got, wanted);
    assert!(!has_key(&dir), "a CI build wrote a key into the checkout");
}

/// A key mangled in a secret store produces packs nothing can open. The
/// error has to name the variable, not the parse.
#[test]
fn a_malformed_environment_key_names_the_variable() {
    let _alone = alone();
    let dir = tmp("badenv");
    unsafe { std::env::set_var(KEY_ENV, "not a key") };

    let err = project_key(&dir).unwrap_err();

    unsafe { std::env::remove_var(KEY_ENV) };
    assert!(
        err.to_string().contains(KEY_ENV),
        "the error does not say where the key came from: {err}",
    );
}

/// A file that is there but unreadable as a key is replaced rather than
/// failing the build: the alternative is a project that cannot build and
/// an error about hex.
#[test]
fn a_corrupt_key_file_is_replaced() {
    let _alone = alone();
    let dir = tmp("corrupt");
    std::fs::create_dir_all(dir.join(LOCAL_DIR)).unwrap();
    std::fs::write(key_path(&dir), "this is not a key").unwrap();

    let key = project_key(&dir).unwrap();

    assert_eq!(
        std::fs::read_to_string(key_path(&dir)).unwrap(),
        key.to_hex()
    );
}

/// The key file is the one secret in the project directory.
#[cfg(unix)]
#[test]
fn the_key_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let _alone = alone();
    let dir = tmp("perms");
    project_key(&dir).unwrap();

    let mode = std::fs::metadata(key_path(&dir))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "mode is {:o}", mode & 0o777);
}
