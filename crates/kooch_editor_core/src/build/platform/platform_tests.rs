use super::*;

/// The two facts a triple used to be searched for.
#[test]
fn a_platform_knows_its_triple_and_extension() {
    assert_eq!(Platform::Windows.triple(), "x86_64-pc-windows-gnu");
    assert_eq!(Platform::Windows.extension(), ".exe");
    assert_eq!(Platform::Linux.triple(), "x86_64-unknown-linux-gnu");
    assert_eq!(Platform::Linux.extension(), ".x86_64");
}

/// A floor must not follow a build onto Windows.
///
/// `cargo zigbuild` spells a floor by appending it to the triple, and
/// `x86_64-pc-windows-gnu.2.28` is not a target — the build would fail
/// on an argument nobody typed.
#[test]
fn only_linux_takes_a_glibc_floor() {
    assert!(Platform::Linux.takes_glibc_floor());
    assert!(!Platform::Windows.takes_glibc_floor());
}

/// Reading an old preset's triple back into a platform.
#[test]
fn a_triple_names_its_platform() {
    assert_eq!(
        Platform::from_triple("x86_64-pc-windows-gnu"),
        Some(Platform::Windows),
    );
    assert_eq!(
        Platform::from_triple("  x86_64-unknown-linux-gnu  "),
        Some(Platform::Linux),
    );
    // Empty means "this machine", which is not a triple to read.
    assert_eq!(Platform::from_triple(""), None);
}

/// Two platforms must never share an output folder, or the second build
/// overwrites the first.
#[test]
fn every_platform_has_its_own_folder() {
    let mut folders: Vec<&str> = Platform::ALL.iter().map(|p| p.folder()).collect();
    folders.sort_unstable();
    let count = folders.len();
    folders.dedup();
    assert_eq!(folders.len(), count, "two platforms share a folder");
}
