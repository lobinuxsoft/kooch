//! What a machine needs before a project it opens can build.
//!
//! A project made with this editor **compiles the engine** — the
//! gameplay is native Rust and links it as an `rlib` — so the machine
//! needs a Rust toolchain and the system libraries the engine's `*-sys`
//! crates link against. Without them cargo runs for minutes and dies
//! with an error that names a crate nobody has heard of.
//!
//! # 🔴 What this refuses to check, and why
//!
//! `build::compile` states the rule this follows: *"Only what is
//! knowable without compiling. A missing C toolchain is not — that
//! surfaces from cargo, and guessing at it would mean refusing builds
//! that would have worked."*
//!
//! So this does not read a list of packages a wiki once recommended. It
//! runs **the same probe the build runs**: `pkg-config --exists alsa` is
//! what `alsa-sys`'s build script does, so its answer is the build's
//! answer rather than a guess about it.
//!
//! That distinction was learned the expensive way. A machine that could
//! not build was diagnosed from a documentation table as missing four
//! `-devel` packages; the actual build failed on exactly one, and the
//! other three are dlopened by winit at runtime and were never needed.
//!
//! # Why it cannot just install them
//!
//! On an image-based distribution — which both of this project's Linux
//! targets are — installing a package writes a new image and needs a
//! **reboot**. That is not something an editor may do behind a dialog.
//! What it can do is say exactly what is missing and the exact command
//! for *this* machine, which is the part that is otherwise a web search.

use std::process::{Command, Stdio};

/// Something the machine needs and the reason it needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requirement {
    /// What to look for, in the vocabulary of whoever installs it.
    pub name: &'static str,
    /// What breaks without it, in terms of what the user was doing.
    pub why: &'static str,
    /// Where it comes from when no package manager provides it. Empty
    /// when the command below is the whole answer.
    pub hint: &'static str,
}

/// The Rust toolchain. Installed the same way on every platform, which
/// is why it carries no per-installer package name.
pub const RUST: Requirement = Requirement {
    name: "Rust",
    why: "a project compiles the engine, so it needs cargo and rustc",
    hint: "https://rustup.rs",
};

/// The official rustup line, for everything that is not Windows.
///
/// 🔴 Not the distribution's `rust` package, even where one exists. A
/// toolchain installed through a package manager cannot be updated by
/// `rustup update`, cannot add a target with `rustup target add` — which
/// `build::compile` tells people to run — and on an image-based system
/// it needs a reboot to change. The line below is what rust-lang.org
/// gives, and it installs into the user's home.
const RUSTUP_UNIX: &str = "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh";

/// Windows has a real package for it, and it is still rustup rather than
/// a toolchain — so it keeps every property the line above has.
const RUSTUP_WINDOWS: &str = "winget install Rustlang.Rustup";

/// ALSA's development files.
///
/// Reached through `alsa-sys` <- `alsa` <- `cpal` <- `kira` <-
/// `kooch_audio`, which is the `audio` feature — on by default in a
/// generated project. It is the one system library in this engine's tree
/// that is linked rather than dlopened, which is why it is the one that
/// stops a build.
pub const ALSA: Requirement = Requirement {
    name: "ALSA development files",
    why: "the audio feature links alsa-sys, and a project enables audio by default",
    hint: "",
};

/// How this machine installs things.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installer {
    /// Fedora Atomic and everything built on it. 🔴 Needs a reboot.
    RpmOstree,
    Dnf,
    Apt,
    Pacman,
    Winget,
    /// Nothing recognised — the requirement is named without a command,
    /// which beats printing one that does not exist here.
    Unknown,
}

impl Installer {
    /// The command that installs `requirements` here, or `None` when
    /// this machine's package manager was not recognised.
    pub fn command(self, requirements: &[Requirement]) -> Option<String> {
        let packages: Vec<&str> = requirements
            .iter()
            .filter_map(|req| self.package(*req))
            .collect();
        if packages.is_empty() {
            return None;
        }
        let packages = packages.join(" ");
        Some(match self {
            Self::RpmOstree => format!("rpm-ostree install {packages}\nsystemctl reboot"),
            Self::Dnf => format!("sudo dnf install {packages}"),
            Self::Apt => format!("sudo apt install {packages}"),
            Self::Pacman => format!("sudo pacman -S {packages}"),
            Self::Winget => format!("winget install {packages}"),
            Self::Unknown => return None,
        })
    }

    /// What this requirement is called here.
    ///
    /// `None` for [`RUST`] everywhere: rustup is not a distribution
    /// package and installing it through one is how a machine ends up
    /// with a toolchain it cannot update.
    fn package(self, requirement: Requirement) -> Option<&'static str> {
        match (self, requirement.name) {
            (Self::RpmOstree | Self::Dnf, "ALSA development files") => Some("alsa-lib-devel"),
            (Self::Apt, "ALSA development files") => Some("libasound2-dev"),
            (Self::Pacman, "ALSA development files") => Some("alsa-lib"),
            _ => None,
        }
    }

    /// 🔴 Reads `ID_LIKE` as well as `ID`, and that is not defensive
    /// programming — it is the case this project's own distribution
    /// hits. YaguareteOS reports `ID=yaguarete` with
    /// `ID_LIKE="bazzite fedora"`, so an `ID`-only match does not
    /// recognise the machine it was built for.
    ///
    /// `atomic` decides between the two Fedora answers, because on an
    /// image-based system `dnf` does not work at all and the command
    /// ends in a reboot.
    pub fn from_os_release(id: &str, id_like: &str, atomic: bool) -> Self {
        let names: Vec<&str> = std::iter::once(id)
            .chain(id_like.split_whitespace())
            .map(str::trim)
            .collect();
        let has = |wanted: &str| names.iter().any(|name| *name == wanted);

        if has("fedora") || has("rhel") || has("bazzite") {
            return match atomic {
                true => Self::RpmOstree,
                false => Self::Dnf,
            };
        }
        if has("debian") || has("ubuntu") {
            return Self::Apt;
        }
        if has("arch") {
            return Self::Pacman;
        }
        Self::Unknown
    }

    /// This machine's, read from the system.
    pub fn detect() -> Self {
        if cfg!(target_os = "windows") {
            return Self::Winget;
        }
        let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let field = |key: &str| {
            release
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .unwrap_or_default()
                .trim_matches(['"', '\''])
                .to_owned()
        };
        // `/run/ostree-booted` is what an image-based boot leaves
        // behind, and it is how rpm-ostree itself decides.
        let atomic = std::path::Path::new("/run/ostree-booted").exists();
        Self::from_os_release(&field("ID="), &field("ID_LIKE="), atomic)
    }
}

/// What the probes found, separated from the decision so a test can
/// state a machine rather than be run on one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probes {
    pub cargo: bool,
    pub alsa: bool,
}

impl Probes {
    /// Runs the probes against this machine.
    ///
    /// Each is the same question the failing step asks: `cargo` answers
    /// whether a build can start at all, and `pkg-config --exists alsa`
    /// is literally what `alsa-sys`'s build script runs.
    pub fn detect() -> Self {
        Self {
            cargo: ran("cargo", &["--version"]),
            alsa: ran("pkg-config", &["--exists", "alsa"]),
        }
    }
}

/// What is missing, most blocking first.
///
/// Rust leads because without it nothing else matters: a machine with no
/// cargo cannot act on a message about ALSA.
pub fn missing_from(probes: Probes) -> Vec<Requirement> {
    let mut missing = Vec::new();
    if !probes.cargo {
        missing.push(RUST);
    }
    if !probes.alsa {
        missing.push(ALSA);
    }
    missing
}

/// What the check found on this machine, as the editor shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub missing: Vec<Requirement>,
    pub installer: Installer,
}

impl Report {
    /// Runs the check. Once per editor launch, at startup — the answer
    /// cannot change while the editor runs, since installing any of it
    /// ends in a reboot.
    pub fn detect() -> Self {
        Self {
            missing: missing_from(Probes::detect()),
            installer: Installer::detect(),
        }
    }

    /// Whether anything is worth showing. A dialog that appears when
    /// there is no problem is one people learn to dismiss unread.
    pub fn is_ready(&self) -> bool {
        self.missing.is_empty()
    }

    /// One block that fixes everything missing, ready to paste.
    ///
    /// 🔴 The order is the point. rustup comes first and the packages
    /// last, because on an image-based system the package step **ends in
    /// a reboot** — anything after it would never run. A correct list of
    /// commands in the wrong order is a machine that comes back up still
    /// missing half of them.
    pub fn command(&self) -> Option<String> {
        let mut steps = Vec::new();
        if self.missing.contains(&RUST) {
            steps.push(match self.installer {
                Installer::Winget => RUSTUP_WINDOWS.to_owned(),
                _ => RUSTUP_UNIX.to_owned(),
            });
        }
        if let Some(packages) = self.installer.command(&self.missing) {
            steps.push(packages);
        }
        (!steps.is_empty()).then(|| steps.join("\n"))
    }
}

fn ran(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests;
