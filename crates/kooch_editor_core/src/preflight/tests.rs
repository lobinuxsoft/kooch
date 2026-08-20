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
