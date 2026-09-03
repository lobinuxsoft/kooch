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

/// The Vulkan headers, which bindgen reads while building `dlss_wgpu`.
///
/// 🔴 Not the loader and not a driver — the HEADERS. A machine that runs
/// Vulkan games perfectly well has no `vulkan/vulkan.h`, because nothing
/// but a compiler ever wants one. That is why this is a separate
/// requirement from anything the engine needs at runtime, and why it is
/// checked before cargo: without it the build dies minutes in, inside
/// bindgen, with a message about a missing include.
pub const VULKAN_HEADERS: Requirement = Requirement {
    name: "Vulkan headers",
    why: "a build with the DLSS feature runs bindgen over NVIDIA's SDK, which includes vulkan/vulkan.h",
    hint: "https://vulkan.lunarg.com/sdk/home — set VULKAN_SDK to where it lands",
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
    pub(crate) fn package(self, requirement: Requirement) -> Option<&'static str> {
        match (self, requirement.name) {
            (Self::RpmOstree | Self::Dnf, "ALSA development files") => Some("alsa-lib-devel"),
            (Self::Apt, "ALSA development files") => Some("libasound2-dev"),
            (Self::Pacman, "ALSA development files") => Some("alsa-lib"),
            (Self::RpmOstree | Self::Dnf | Self::Pacman, "Vulkan headers") => {
                Some("vulkan-headers")
            }
            (Self::Apt, "Vulkan headers") => Some("libvulkan-dev"),
            // udev lives in systemd's development package on Fedora and
            // Arch, and in one of its own on Debian.
            (Self::RpmOstree | Self::Dnf, "udev development files") => Some("systemd-devel"),
            (Self::Apt, "udev development files") => Some("libudev-dev"),
            (Self::Pacman, "udev development files") => Some("systemd-libs"),
            (Self::RpmOstree | Self::Dnf, "A C compiler") => Some("gcc"),
            (Self::Apt, "A C compiler") => Some("build-essential"),
            (Self::Pacman, "A C compiler") => Some("base-devel"),
            (Self::RpmOstree | Self::Dnf | Self::Pacman, "mold (faster linker)") => Some("mold"),
            (Self::Apt, "mold (faster linker)") => Some("mold"),
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

/// The udev development files, which `libudev-sys` links.
///
/// 🔴 Found by auditing the dependency tree, not a wiki: `libudev-sys`
/// is pulled by `gilrs-core`, which is how this engine sees a gamepad.
/// Without it the build fails — and this engine's character controller
/// is steered with a stick, so it is not an optional corner.
pub const UDEV: Requirement = Requirement {
    name: "udev development files",
    why: "gamepads are read through libudev, which `gilrs` links against",
    hint: "",
};

/// A C compiler, for the crates that build C rather than bind to it.
///
/// `metis-sys` and `zstd-sys` compile their own sources, so a machine
/// with no `cc` fails partway through a build that looked like it was
/// working.
///
/// ⚠️ Probed on Linux only. On Windows the compiler is MSVC, found
/// through `vswhere` rather than on `PATH`, and answering that question
/// wrongly would refuse builds that work — which is the failure this
/// module exists to avoid.
pub const C_COMPILER: Requirement = Requirement {
    name: "A C compiler",
    why: "some dependencies build C sources rather than link a library",
    hint: "",
};

/// A faster linker. **Not required** — the build works without it.
///
/// 🔴 Listed anyway, and only in the optional half, for the reason this
/// whole check exists: on an image-based system every package found out
/// about later costs another reboot. One command, pasted once.
///
/// Measured on this project: a one-line change relinks a 645 MB binary,
/// and that link is where the 15 s goes — not the compile.
pub const MOLD: Requirement = Requirement {
    name: "mold (faster linker)",
    why: "a rebuild spends most of its time linking, and mold cuts that several-fold",
    hint: "",
};

/// What the probes found, separated from the decision so a test can
/// state a machine rather than be run on one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probes {
    pub cargo: bool,
    pub alsa: bool,
    pub vulkan_headers: bool,
    pub udev: bool,
    /// `true` off Linux, where the question does not apply and a wrong
    /// answer would refuse a build that works.
    pub c_compiler: bool,
    /// Not required. Absent means slower, never broken.
    pub mold: bool,
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
            vulkan_headers: vulkan_header().is_file(),
            // The same query `libudev-sys`'s build script makes.
            udev: !cfg!(target_os = "linux") || ran("pkg-config", &["--exists", "libudev"]),
            // The same binary the `cc` crate runs. Not a guess about a
            // toolchain — the toolchain itself, answering.
            c_compiler: !cfg!(target_os = "linux") || ran("cc", &["--version"]),
            mold: ran("mold", &["--version"]),
        }
    }
}

/// The header, where `dlss_wgpu`'s build script looks for it.
///
/// 🔴 A file test rather than `pkg-config --exists vulkan`: that answers
/// for the LOADER, which is on every machine that runs a game and says
/// nothing about whether a compiler could find `vulkan/vulkan.h`. Asking
/// the wrong question is how this requirement was missed the first time.
pub fn vulkan_header() -> std::path::PathBuf {
    let root = std::env::var_os("VULKAN_SDK")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/usr"));
    let include = if cfg!(windows) { "Include" } else { "include" };
    root.join(include).join("vulkan").join("vulkan.h")
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
    // Last, because it is the only one that is not needed to open a
    // project — and it is here anyway. The whole point of this check is
    // ONE command, pasted once: on an image-based system, finding out
    // about a package later costs another reboot.
    if !probes.udev {
        missing.push(UDEV);
    }
    if !probes.c_compiler {
        missing.push(C_COMPILER);
    }
    if !probes.vulkan_headers {
        missing.push(VULKAN_HEADERS);
    }
    missing
}

/// What would help and is not required.
///
/// Kept apart from [`missing_from`] on purpose. A list that mixes "you
/// cannot build without this" with "this would be faster" is one people
/// read diagonally — and reading it diagonally is how three of the four
/// packages in the story above got installed for nothing.
pub fn wanted_from(probes: Probes) -> Vec<Requirement> {
    let mut wanted = Vec::new();
    if !probes.mold && cfg!(target_os = "linux") {
        wanted.push(MOLD);
    }
    wanted
}

/// What the check found on this machine, as the editor shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub missing: Vec<Requirement>,
    /// Not required, and offered in the same command so the one reboot
    /// covers them. See [`wanted_from`].
    pub wanted: Vec<Requirement>,
    pub installer: Installer,
}

impl Report {
    /// Runs the check. Once per editor launch, at startup — the answer
    /// cannot change while the editor runs, since installing any of it
    /// ends in a reboot.
    pub fn detect() -> Self {
        let probes = Probes::detect();
        Self {
            missing: missing_from(probes),
            wanted: wanted_from(probes),
            installer: Installer::detect(),
        }
    }

    /// Whether anything is worth showing. A dialog that appears when
    /// there is no problem is one people learn to dismiss unread.
    pub fn is_ready(&self) -> bool {
        self.missing.is_empty()
    }

    /// Whether the window has anything to say at all — a machine that
    /// can build but would build faster still has one thing to offer.
    pub fn is_quiet(&self) -> bool {
        self.missing.is_empty() && self.wanted.is_empty()
    }

    /// Everything to install, required first.
    fn all(&self) -> Vec<Requirement> {
        let mut all = self.missing.clone();
        all.extend(self.wanted.iter().copied());
        all
    }

    /// Whether this machine's package step ends in a restart.
    pub fn reboots(&self) -> bool {
        self.installer == Installer::RpmOstree
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
        if let Some(packages) = self.installer.command(&self.all()) {
            steps.push(packages);
        }
        (!steps.is_empty()).then(|| steps.join("\n"))
    }

    /// The package step alone, without the `rustup` line or the reboot.
    ///
    /// What [`crate::install`] hands to the privileged helper: rustup is
    /// a per-user install that must **not** run as root, and the restart
    /// is issued separately so it can be refused when the editor holds
    /// unsaved work.
    pub fn packages(&self) -> Option<String> {
        let all = self.all();
        let packages: Vec<&str> = all
            .iter()
            .filter_map(|req| self.installer.package(*req))
            .collect();
        (!packages.is_empty()).then(|| packages.join(" "))
    }

    /// Whether [`RUST`] is missing, which the editor cannot install for
    /// you: rustup installs into the invoking user's home, and running
    /// it through a privileged helper would put a toolchain in root's.
    pub fn needs_rust(&self) -> bool {
        self.missing.contains(&RUST)
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
