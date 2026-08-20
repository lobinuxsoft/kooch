use super::{ALSA, Installer, Probes, RUST, missing_from};

/// 🔴 The case this project's own distribution hits: YaguareteOS reports
/// `ID=yaguarete` and only names Fedora in `ID_LIKE`. An `ID`-only match
/// would not recognise the machine the engine is built for.
#[test]
fn id_like_is_read_too() {
    assert_eq!(
        Installer::from_os_release("yaguarete", "bazzite fedora", true),
        Installer::RpmOstree,
    );
    assert_eq!(
        Installer::from_os_release("bazzite", "fedora", true),
        Installer::RpmOstree,
    );
}

/// 🔴 On an image-based system `dnf` does not work at all, so the two
/// Fedora answers are not interchangeable.
#[test]
fn atomic_and_classic_differ() {
    assert_eq!(
        Installer::from_os_release("fedora", "", true),
        Installer::RpmOstree
    );
    assert_eq!(
        Installer::from_os_release("fedora", "", false),
        Installer::Dnf
    );
}

#[test]
fn the_other_families_are_known() {
    assert_eq!(
        Installer::from_os_release("ubuntu", "debian", false),
        Installer::Apt
    );
    assert_eq!(
        Installer::from_os_release("arch", "", false),
        Installer::Pacman
    );
}

/// An unrecognised machine names the requirement and offers no command,
/// which beats printing one that does not exist here.
#[test]
fn an_unknown_distro_offers_nothing() {
    let installer = Installer::from_os_release("plan9", "", false);
    assert_eq!(installer, Installer::Unknown);
    assert_eq!(installer.command(&[ALSA]), None);
}

/// The package name differs per family and getting it wrong is a command
/// that fails in front of somebody who was already stuck.
#[test]
fn each_family_names_alsa() {
    let of = |installer: Installer| installer.command(&[ALSA]).unwrap_or_default();
    assert!(of(Installer::RpmOstree).contains("alsa-lib-devel"));
    assert!(of(Installer::Dnf).contains("alsa-lib-devel"));
    assert!(of(Installer::Apt).contains("libasound2-dev"));
    assert!(of(Installer::Pacman).contains("alsa-lib"));
}

/// 🔴 An image-based install is not complete until the machine reboots,
/// and a command that omits it leaves somebody wondering why nothing
/// changed.
#[test]
fn the_atomic_command_reboots() {
    let command = Installer::RpmOstree.command(&[ALSA]).expect("a command");
    assert!(command.contains("rpm-ostree install"));
    assert!(command.contains("reboot"), "no reboot in: {command}");
}

/// 🔴 Rust is never a distribution package. Installing it through one is
/// how a machine ends up with a toolchain it cannot update, so it is
/// named without a command rather than given a wrong one.
#[test]
fn rust_is_never_a_package() {
    for installer in [
        Installer::RpmOstree,
        Installer::Dnf,
        Installer::Apt,
        Installer::Pacman,
        Installer::Winget,
    ] {
        assert_eq!(
            installer.command(&[RUST]),
            None,
            "{installer:?} offered one"
        );
    }
}

/// A machine with everything says nothing — a dialog that appears when
/// there is no problem is one people learn to dismiss unread.
#[test]
fn a_ready_machine_reports_nothing() {
    assert!(
        missing_from(Probes {
            cargo: true,
            alsa: true
        })
        .is_empty()
    );
}

/// Rust leads: a machine with no cargo cannot act on a message about
/// ALSA.
#[test]
fn rust_is_reported_first() {
    let missing = missing_from(Probes {
        cargo: false,
        alsa: false,
    });
    assert_eq!(missing, vec![RUST, ALSA]);
}

use super::Report;

fn report(missing: Vec<super::Requirement>, installer: Installer) -> Report {
    Report { missing, installer }
}

/// The whole point of the dialog: one block, not a list of things to go
/// and find. A machine missing everything gets rustup and the packages
/// together.
#[test]
fn one_block_fixes_everything() {
    let command = report(vec![RUST, ALSA], Installer::RpmOstree)
        .command()
        .expect("a command");
    assert!(command.contains("sh.rustup.rs"), "no rustup in: {command}");
    assert!(
        command.contains("alsa-lib-devel"),
        "no package in: {command}"
    );
}

/// 🔴 On an image-based system the package step ENDS IN A REBOOT, so
/// anything after it never runs. A correct list in the wrong order is a
/// machine that comes back up still missing half of it.
#[test]
fn the_reboot_is_the_last_line() {
    let command = report(vec![RUST, ALSA], Installer::RpmOstree)
        .command()
        .expect("a command");
    let lines: Vec<&str> = command.lines().collect();
    assert!(
        lines.last().is_some_and(|last| last.contains("reboot")),
        "the reboot is not last: {command}",
    );
    let rustup = lines
        .iter()
        .position(|l| l.contains("rustup"))
        .expect("rustup");
    let install = lines
        .iter()
        .position(|l| l.contains("rpm-ostree"))
        .expect("packages");
    assert!(rustup < install, "rustup runs after the reboot: {command}");
}

/// Windows installs rustup through winget, and it is still rustup — not
/// a toolchain a package manager owns.
#[test]
fn windows_gets_rustup_too() {
    let command = report(vec![RUST], Installer::Winget)
        .command()
        .expect("a command");
    assert!(command.contains("Rustlang.Rustup"), "got: {command}");
}

/// A machine whose package manager is unknown still gets the half that
/// does not depend on one.
#[test]
fn an_unknown_distro_still_installs_rust() {
    let command = report(vec![RUST, ALSA], Installer::Unknown)
        .command()
        .expect("a command");
    assert!(command.contains("sh.rustup.rs"));
    assert!(
        !command.contains("alsa"),
        "invented a package command: {command}"
    );
}

/// Nothing missing, nothing to paste.
#[test]
fn a_ready_machine_has_no_command() {
    assert_eq!(report(vec![], Installer::RpmOstree).command(), None);
}
