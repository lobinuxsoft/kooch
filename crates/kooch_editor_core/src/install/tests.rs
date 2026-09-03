use kooch_core::resource::Resources;

use crate::preflight::{ALSA, Installer, MOLD, RUST, Report, UDEV};

use super::{Refusal, privileged, refusal};

fn report(installer: Installer, missing: &[crate::preflight::Requirement]) -> Report {
    Report {
        missing: missing.to_vec(),
        wanted: Vec::new(),
        installer,
    }
}

/// 🔴 The refusal that matters. Everything else here is an
/// inconvenience; this one is the editor restarting the machine out from
/// under work that only exists in memory.
#[test]
fn unsaved_work_refuses_the_restart() {
    let mut manager = kooch_ecs::SceneManager::new();
    manager.mark_dirty();
    let mut resources = Resources::new();
    resources.insert(manager);

    let report = report(Installer::RpmOstree, &[ALSA]);

    assert_eq!(
        refusal(&resources, &report),
        Some(Refusal::Unsaved),
        "the editor was about to reboot over unsaved scenes",
    );
}

#[test]
fn a_clean_editor_may_install() {
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::SceneManager::new());

    assert_eq!(
        refusal(&resources, &report(Installer::RpmOstree, &[ALSA])),
        None
    );
}

/// An unrecognised package manager has no command, and offering the
/// button would mean failing at the click instead of never offering it.
#[test]
fn an_unknown_installer_refuses() {
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::SceneManager::new());

    assert_eq!(
        refusal(&resources, &report(Installer::Unknown, &[ALSA])),
        Some(Refusal::NoInstaller),
    );
}

/// 🔴 Rust is not a package here, so a report holding only Rust has
/// nothing to install — offering to install it would put a toolchain in
/// root's home and leave the user's shell finding nothing.
#[test]
fn rust_alone_installs_nothing() {
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::SceneManager::new());

    assert_eq!(
        refusal(&resources, &report(Installer::RpmOstree, &[RUST])),
        Some(Refusal::Nothing),
    );
}

/// Every recognised installer raises the system's own authentication
/// rather than asking for a password itself.
#[test]
fn every_installer_escalates_through_the_system() {
    for installer in [
        Installer::RpmOstree,
        Installer::Dnf,
        Installer::Apt,
        Installer::Pacman,
    ] {
        let argv = privileged(installer, "alsa-lib-devel").expect("a command");
        assert_eq!(
            argv[0], "pkexec",
            "{installer:?} escalated some other way than the desktop's own prompt",
        );
    }
    assert!(privileged(Installer::Unknown, "anything").is_none());
}

/// The optional half rides along in the same command, because on an
/// image-based system a package found out about later costs a reboot.
#[test]
fn the_optional_half_rides_along() {
    let report = Report {
        missing: vec![UDEV],
        wanted: vec![MOLD],
        installer: Installer::RpmOstree,
    };
    let packages = report.packages().expect("packages");
    assert!(packages.contains("systemd-devel"), "{packages}");
    assert!(packages.contains("mold"), "{packages}");
}
