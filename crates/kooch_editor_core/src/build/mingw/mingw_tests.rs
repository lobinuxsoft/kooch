use super::*;

/// A Linux build carries no Windows runtime.
#[test]
fn only_a_windows_build_carries_the_runtime() {
    let dir = std::env::temp_dir().join("kooch_mingw_linux");
    let _ = std::fs::create_dir_all(&dir);

    assert_eq!(ship(Platform::Linux, &dir).unwrap(), Vec::<PathBuf>::new());
    assert!(
        std::fs::read_dir(&dir).unwrap().next().is_none(),
        "a Linux build gained Windows DLLs",
    );
}

/// 🔴 All three, or the executable does not start.
///
/// `libstdc++-6.dll` is the one the linker asks for, and it in turn
/// needs `libgcc_s_seh-1.dll` and `libwinpthread-1.dll`. Shipping only
/// the first produces the same Windows dialog one file later.
#[test]
fn a_windows_build_carries_the_whole_chain() {
    let Some(_) = runtime_dir() else {
        // No mingw on this machine; `ship` would fail with the message
        // that says so, which `a_missing_runtime_is_an_error` covers.
        return;
    };
    let dir = std::env::temp_dir().join("kooch_mingw_windows");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let written = ship(Platform::Windows, &dir).expect("the runtime is installed");

    let names: Vec<String> = written
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    for needed in RUNTIME {
        assert!(names.iter().any(|n| n == needed), "{needed} did not travel");
    }
    assert!(written.iter().all(|p| p.is_file()));
}

/// And stripping actually happens: the copy is far smaller than the one
/// Fedora installs, which carries its debug symbols.
#[test]
fn the_copied_runtime_is_stripped() {
    let Some(from) = runtime_dir() else {
        return;
    };
    let installed = std::fs::metadata(from.join("libstdc++-6.dll"))
        .map(|m| m.len())
        .unwrap_or(0);
    // Nothing to prove on a distribution that ships it stripped already.
    if installed < 8_000_000 {
        return;
    }
    let dir = std::env::temp_dir().join("kooch_mingw_strip");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    ship(Platform::Windows, &dir).expect("the runtime is installed");

    let shipped = std::fs::metadata(dir.join("libstdc++-6.dll"))
        .unwrap()
        .len();
    assert!(
        shipped * 4 < installed,
        "the DLL travelled with its debug symbols: {shipped} vs {installed}",
    );
}
