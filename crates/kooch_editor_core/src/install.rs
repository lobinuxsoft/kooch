//! Installing what a machine is missing, from the editor.
//!
//! # 🔴 This restarts the computer, and that is the point
//!
//! On an image-based distribution a package writes a new image, and the
//! new image is not the running one until a reboot. So an install that
//! stops short of restarting has not finished — the editor would report
//! success and the next build would fail exactly as before.
//!
//! [`preflight`](crate::preflight) used to say this was "not something
//! an editor may do behind a dialog", and it was right about the dialog.
//! What changed is not the risk but where the decision sits: the owner
//! of this engine asked for it, and it happens **in front of** a dialog
//! that names the command, names the restart, and refuses while there is
//! unsaved work.
//!
//! # What it will not do
//!
//! - **Run with unsaved scenes open.** A restart with a dirty scene is
//!   the editor destroying the author's work to save them a paste.
//! - **Install Rust.** `rustup` installs into the invoking user's home;
//!   through a privileged helper it would put a toolchain in root's, and
//!   the user's shell would still find nothing.
//! - **Escalate silently.** `pkexec` puts the authentication where the
//!   system puts it, which is the desktop's own prompt and not ours.

use std::process::Command;

use kooch_core::resource::Resources;

use crate::preflight::{Installer, Report};

/// Why an install cannot start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Scenes hold edits that a restart would take with them.
    Unsaved,
    /// No package manager was recognised, so there is no command.
    NoInstaller,
    /// Nothing to install.
    Nothing,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsaved => write!(
                f,
                "save your open scenes first — this restarts the machine, and \
                 unsaved work does not survive that"
            ),
            Self::NoInstaller => write!(
                f,
                "this machine's package manager was not recognised, so there is \
                 no command to run"
            ),
            Self::Nothing => write!(f, "there is nothing to install"),
        }
    }
}

/// Whether an install may start right now.
///
/// Separated from running it so the button can be disabled with the
/// reason showing, rather than offered and then refused.
pub fn refusal(resources: &Resources, report: &Report) -> Option<Refusal> {
    if report.packages().is_none() {
        return Some(match report.installer {
            Installer::Unknown => Refusal::NoInstaller,
            _ => Refusal::Nothing,
        });
    }
    // 🔴 The one that matters. Everything else is an inconvenience.
    let dirty = resources
        .get::<kooch_ecs::SceneManager>()
        .is_some_and(kooch_ecs::SceneManager::any_dirty);
    dirty.then_some(Refusal::Unsaved)
}

/// The privileged command that installs `packages` here.
///
/// `pkexec` on Linux, which raises the desktop's own authentication
/// dialog. `winget` on Windows elevates itself.
pub fn privileged(installer: Installer, packages: &str) -> Option<Vec<String>> {
    let owned = |parts: &[&str]| Some(parts.iter().map(|s| (*s).to_owned()).collect());
    match installer {
        Installer::RpmOstree => owned(&["pkexec", "rpm-ostree", "install", "-y", packages]),
        Installer::Dnf => owned(&["pkexec", "dnf", "install", "-y", packages]),
        Installer::Apt => owned(&["pkexec", "apt-get", "install", "-y", packages]),
        Installer::Pacman => owned(&["pkexec", "pacman", "-S", "--noconfirm", packages]),
        Installer::Winget => owned(&["winget", "install", packages]),
        Installer::Unknown => None,
    }
}

/// The command that restarts this machine.
fn restart() -> Vec<String> {
    match cfg!(target_os = "windows") {
        true => vec!["shutdown".into(), "/r".into(), "/t".into(), "5".into()],
        false => vec!["pkexec".into(), "systemctl".into(), "reboot".into()],
    }
}

/// Installs what is missing and restarts, unless something refuses.
///
/// Returns the refusal rather than acting on it, so the caller can say
/// so where the button is.
pub fn run(resources: &mut Resources, report: &Report) -> Result<(), Refusal> {
    if let Some(refusal) = refusal(resources, report) {
        return Err(refusal);
    }
    let packages = report.packages().ok_or(Refusal::Nothing)?;
    let argv = privileged(report.installer, &packages).ok_or(Refusal::NoInstaller)?;

    tracing::info!(command = %argv.join(" "), "installing what this machine is missing");
    let installed = Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .is_ok_and(|status| status.success());
    if !installed {
        // 🔴 No restart. A failed install followed by a reboot is a
        // machine that comes back up no better and a user who watched it
        // happen for nothing — and on an atomic system the failure is
        // routinely "that package does not exist", which a reboot hides.
        tracing::error!(
            "the install did not finish — nothing was restarted. Run the command \
             yourself to see what it said",
        );
        return Ok(());
    }

    if report.needs_rust() {
        tracing::warn!(
            "Rust is still missing and this cannot install it: rustup installs into \
             your own home, and through a privileged helper it would land in root's. \
             Run the rustup line from the window after the restart",
        );
    }

    let argv = restart();
    tracing::warn!(command = %argv.join(" "), "restarting — the new image is not the running one until then");
    if let Err(e) = Command::new(&argv[0]).args(&argv[1..]).status() {
        tracing::error!(error = %e, "the restart could not be issued; restart the machine yourself");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
