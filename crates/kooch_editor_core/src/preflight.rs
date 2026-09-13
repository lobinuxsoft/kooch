//! What a machine needs before a project it opens can build.

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
const RUSTUP_UNIX: &str = "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh";

/// Windows has a real package for it, and it is still rustup rather than
/// a toolchain — so it keeps every property the line above has.
const RUSTUP_WINDOWS: &str = "winget install Rustlang.Rustup";

/// ALSA's development files.
pub const ALSA: Requirement = Requirement {
    name: "ALSA development files",
    why: "the audio feature links alsa-sys, and a project enables audio by default",
    hint: "",
};

/// The Vulkan headers, which bindgen reads while building `dlss_wgpu`.
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

    /// 🔴 Reads `ID_LIKE` as well as `ID`, and that is not defensive programming — it is the case
    /// this project's own distribution hits.
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
pub const UDEV: Requirement = Requirement {
    name: "udev development files",
    why: "gamepads are read through libudev, which `gilrs` links against",
    hint: "",
};

/// A C compiler, for the crates that build C rather than bind to it.
pub const C_COMPILER: Requirement = Requirement {
    name: "A C compiler",
    why: "some dependencies build C sources rather than link a library",
    hint: "",
};

/// A faster linker. **Not required** — the build works without it.
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
pub fn vulkan_header() -> std::path::PathBuf {
    let root = std::env::var_os("VULKAN_SDK")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/usr"));
    let include = if cfg!(windows) { "Include" } else { "include" };
    root.join(include).join("vulkan").join("vulkan.h")
}

/// What is missing, most blocking first.
pub fn missing_from(probes: Probes) -> Vec<Requirement> {
    let mut missing = Vec::new();
    if !probes.cargo {
        missing.push(RUST);
    }
    if !probes.alsa {
        missing.push(ALSA);
    }
    // Last, because it is the only one that is not needed to open a project — and it is here
    // anyway. The whole point of this check is ONE command, pasted once: on an image-based system,
    // finding out about a package later costs another reboot.
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
