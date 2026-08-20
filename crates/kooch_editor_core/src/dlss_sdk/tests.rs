use std::fs;
use std::path::{Path, PathBuf};

use super::{LICENSE, REPO, TAG, VERSION, clone_args, is_installed, runtime_path};

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_dlss_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Lays out what NVIDIA's repository actually contains at the tag, read
/// from its tree rather than guessed.
fn fake_sdk(root: &Path) {
    fs::create_dir_all(root.join("include")).unwrap();
    fs::create_dir_all(root.join("lib/Linux_x86_64/rel")).unwrap();
    fs::write(root.join("include/nvsdk_ngx_helpers.h"), "// ngx").unwrap();
    fs::write(root.join("lib/Linux_x86_64/libnvsdk_ngx.a"), "ar").unwrap();
    fs::write(runtime_path(root), "elf").unwrap();
}

/// 🔴 The version is pinned to the crate, not to whatever NVIDIA tagged
/// last. `dlss_wgpu` 4.0.0's own chart lines up SDK v310.5.3 with wgpu
/// 29, which is the wgpu this engine uses — a newer SDK is a different
/// row of that table.
#[test]
fn the_version_matches_the_crate() {
    assert_eq!(VERSION, "310.5.3");
    assert_eq!(TAG, "v310.5.3");
    assert!(
        LICENSE.contains(TAG),
        "the licence link drifted from the tag"
    );
}

/// 🔴 From NVIDIA and nowhere else. Serving it from a mirror of ours is
/// the "stand-alone product" the licence forbids.
#[test]
fn it_is_fetched_from_nvidia() {
    assert_eq!(REPO, "https://github.com/NVIDIA/DLSS");
    let args = clone_args(Path::new("/tmp/x"));
    assert!(args.contains(&REPO.to_owned()), "got: {args:?}");
    assert!(
        args.contains(&TAG.to_owned()),
        "the tag is not pinned: {args:?}"
    );
    assert!(
        args.contains(&"--depth".to_owned()),
        "cloning the whole history"
    );
}

/// 🔴 An interrupted clone leaves a directory behind, so existence is not
/// the question — the build needs headers for bindgen and the static
/// library it links, and a game needs the runtime.
#[test]
fn a_half_clone_is_not_installed() {
    let dir = tmp("partial");
    assert!(!is_installed(&dir), "an empty directory passed");

    fs::create_dir_all(dir.join("include")).unwrap();
    fs::write(dir.join("include/nvsdk_ngx_helpers.h"), "// ngx").unwrap();
    assert!(!is_installed(&dir), "headers alone passed");

    fs::create_dir_all(dir.join("lib/Linux_x86_64")).unwrap();
    fs::write(dir.join("lib/Linux_x86_64/libnvsdk_ngx.a"), "ar").unwrap();
    assert!(
        !is_installed(&dir),
        "no runtime, and that is what a game ships"
    );
}

#[test]
fn a_whole_sdk_is_installed() {
    let dir = tmp("whole");
    fake_sdk(&dir);
    assert!(is_installed(&dir));
}

/// The runtime carries the version in its file name, and a game copies
/// it by that name — getting it wrong ships nothing and says nothing.
#[test]
fn the_runtime_is_versioned() {
    let path = runtime_path(Path::new("/sdk"));
    assert_eq!(
        path,
        Path::new("/sdk/lib/Linux_x86_64/rel/libnvidia-ngx-dlss.so.310.5.3"),
    );
}

use super::{SdkInstall, SdkState};

/// 🔴 The licence is accepted by USE, so the moment the editor puts the
/// SDK on disk somebody has accepted it. Nothing may start until the
/// person at the keyboard says so.
#[test]
fn nothing_fetches_unaccepted() {
    let mut install = SdkInstall {
        state: Some(SdkState::Missing(PathBuf::from("/tmp/nowhere"))),
        accepted: false,
        ..Default::default()
    };
    assert!(!install.can_fetch());
    install.accepted = true;
    assert!(install.can_fetch(), "accepted and still refusing");
}

/// An SDK already there is not fetched again — the click would delete a
/// working copy to download 55 MB of the same bytes.
#[test]
fn an_installed_sdk_is_not_refetched() {
    let mut install = SdkInstall {
        state: Some(SdkState::Installed(PathBuf::from("/tmp/there"))),
        accepted: true,
        ..Default::default()
    };
    assert!(!install.can_fetch());
}

/// A platform with no data directory has nowhere to put it, and saying
/// so beats a button that does nothing.
#[test]
fn nowhere_cannot_fetch() {
    let mut install = SdkInstall {
        state: Some(SdkState::Nowhere),
        accepted: true,
        ..Default::default()
    };
    assert!(!install.can_fetch());
}

/// A failure may be retried, because the reasons are things that pass:
/// no network, a full disk.
#[test]
fn a_failure_may_be_retried() {
    let mut install = SdkInstall {
        state: Some(SdkState::Failed("no network".to_owned())),
        accepted: true,
        ..Default::default()
    };
    assert!(install.can_fetch());
}
