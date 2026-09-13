//! What a build preset targets, as a platform rather than a triple.

/// A platform a preset can build for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Platform {
    Linux,
    Windows,
}

impl Platform {
    /// Every platform, in the order a build runs them.
    pub const ALL: [Platform; 2] = [Platform::Linux, Platform::Windows];

    /// The triple cargo is told to build for.
    pub fn triple(self) -> &'static str {
        match self {
            Platform::Linux => "x86_64-unknown-linux-gnu",
            Platform::Windows => "x86_64-pc-windows-gnu",
        }
    }

    /// The subfolder of the preset's output directory this lands in.
    pub fn folder(self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::Windows => "windows",
        }
    }

    /// What the platform is called in the editor.
    pub fn label(self) -> &'static str {
        match self {
            Platform::Linux => "Linux",
            Platform::Windows => "Windows",
        }
    }

    /// The extension the executable takes, `""` for none.
    pub fn extension(self) -> &'static str {
        match self {
            // Fixed rather than read from the triple: every variant here
            // is x86_64 by construction, so there is nothing to read.
            Platform::Linux => ".x86_64",
            Platform::Windows => ".exe",
        }
    }

    /// Whether a glibc floor means anything for this platform.
    pub fn takes_glibc_floor(self) -> bool {
        matches!(self, Platform::Linux)
    }

    /// The platform the editor is running on, `None` on one this cannot
    /// build for at all.
    pub fn host() -> Option<Self> {
        match () {
            _ if cfg!(target_os = "linux") => Some(Platform::Linux),
            _ if cfg!(target_os = "windows") => Some(Platform::Windows),
            _ => None,
        }
    }

    /// The platform a target triple names.
    pub fn from_triple(triple: &str) -> Option<Self> {
        let triple = triple.trim();
        match () {
            _ if triple.contains("windows") => Some(Platform::Windows),
            _ if triple.contains("linux") => Some(Platform::Linux),
            _ => None,
        }
    }
}

#[cfg(test)]
mod platform_tests;
