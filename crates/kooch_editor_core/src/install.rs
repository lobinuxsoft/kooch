//! Installing what a machine is missing, from the editor.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

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

/// What an install looks like from the UI, owned so the window can hold
/// it while `Resources` is borrowed.
pub struct Progress {
    pub status: &'static str,
    pub lines: Vec<String>,
    pub running: bool,
}

/// A running install, and everything it has said so far.
pub struct Installing {
    child: Child,
    output: Arc<Mutex<Vec<String>>>,
    /// Every line so far, for the window to draw.
    pub lines: Vec<String>,
    /// `None` while it runs.
    pub finished: Option<bool>,
    /// Whether a restart follows a successful finish.
    pub restarts: bool,
}

impl Installing {
    /// Drains what the child has said and notices when it exits.
    pub fn poll(&mut self) -> bool {
        if let Ok(mut buffered) = self.output.lock() {
            self.lines.append(&mut buffered);
        }
        if self.finished.is_some() {
            return false;
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.finished = Some(status.success());
                true
            }
            Ok(None) => false,
            Err(e) => {
                self.lines
                    .push(format!("could not wait on the installer: {e}"));
                self.finished = Some(false);
                true
            }
        }
    }

    /// What the window needs, owned.
    pub fn progress(&self) -> Progress {
        Progress {
            status: self.status(),
            lines: self.lines.clone(),
            running: self.finished.is_none(),
        }
    }

    /// What to show above the output.
    pub fn status(&self) -> &'static str {
        match self.finished {
            None => "Installing… this writes a system image and takes minutes.",
            Some(true) if self.restarts => "Done. Restarting.",
            Some(true) => "Done.",
            Some(false) => "The install did not finish. Nothing was restarted.",
        }
    }
}

/// Starts the install, unless something refuses.
///
/// Returns immediately: the work is a child process the editor polls.
pub fn start(resources: &mut Resources, report: &Report) -> Result<(), Refusal> {
    if let Some(refusal) = refusal(resources, report) {
        return Err(refusal);
    }
    let packages = report.packages().ok_or(Refusal::Nothing)?;
    let argv = privileged(report.installer, &packages).ok_or(Refusal::NoInstaller)?;

    tracing::info!(command = %argv.join(" "), "installing what this machine is missing");
    let output: Arc<Mutex<Vec<String>>> = Default::default();
    let spawned = Command::new(&argv[0])
        .args(&argv[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            tracing::error!(error = %e, "the installer could not be started");
            return Ok(());
        }
    };
    read_into(&mut child, &output);

    resources.insert(Installing {
        child,
        output,
        lines: vec![argv.join(" ")],
        finished: None,
        restarts: report.reboots(),
    });
    Ok(())
}

/// Drains the installer's output and restarts when it succeeds.
pub fn poll_install_system(resources: &mut Resources) {
    let Some(mut installing) = resources.remove::<Installing>() else {
        return;
    };
    let just_finished = installing.poll();
    let restarts = installing.restarts;
    let succeeded = installing.finished == Some(true);
    // Kept so the window can show the result rather than blinking out.
    resources.insert(installing);

    if !just_finished {
        return;
    }
    if !succeeded {
        // 🔴 No restart. A failed install followed by a reboot is a machine that comes back up no
        // better and a user who watched it happen for nothing — and on an atomic system the failure
        // is routinely "that package does not exist", which a reboot hides.
        tracing::error!("the install did not finish — nothing was restarted");
        return;
    }
    if !restarts {
        tracing::info!("installed");
        return;
    }
    let argv = restart();
    tracing::warn!(command = %argv.join(" "), "restarting — the new image is not the running one until then");
    if let Err(e) = Command::new(&argv[0]).args(&argv[1..]).status() {
        tracing::error!(error = %e, "the restart could not be issued; restart the machine yourself");
    }
}

/// Spawns reader threads that drain the child into the shared buffer.
fn read_into(child: &mut Child, output: &Arc<Mutex<Vec<String>>>) {
    let drain = |stream: Option<Box<dyn std::io::Read + Send>>| {
        let Some(stream) = stream else { return };
        let out = Arc::clone(output);
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if let Ok(mut buffered) = out.lock() {
                    buffered.push(line);
                }
            }
        });
    };
    drain(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    );
    drain(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    );
}

#[cfg(test)]
mod tests;
