//! Finding the IDE this machine actually has.

use std::path::{Path, PathBuf};

/// A launchable IDE: a program and the arguments that come before ours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IdeCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl IdeCommand {
    /// Parses a whitespace-separated command, e.g. `flatpak run com.vscodium.codium`.
    pub(crate) fn parse(command: &str) -> Option<Self> {
        let mut parts = command
            .split_whitespace()
            .map(|part| part.trim_matches(['"', '\'']))
            .filter(|part| !part.is_empty());
        let program = parts.next()?.to_owned();
        Some(Self {
            program,
            args: parts.map(str::to_owned).collect(),
        })
    }

    /// Whether this is a VS Code derivative, which is what decides if `-g <file>` means anything.
    pub(crate) fn understands_goto(&self) -> bool {
        let name = Path::new(&self.program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.program);
        matches!(
            name,
            "code" | "codium" | "code-oss" | "code-insiders" | "cursor" | "windsurf" | "vscodium"
        ) || self.args.iter().any(|arg| {
            // `flatpak run com.visualstudio.code` — the id carries it.
            arg.contains("vscodium")
                || arg.contains("visualstudio.code")
                || arg.contains("VSCodium")
        })
    }
}

/// The IDE the desktop environment would use for a source file.
pub(crate) fn from_desktop_defaults() -> Option<IdeCommand> {
    for mime in ["text/x-rust", "text/rust", "text/plain"] {
        let Some(entry) = default_desktop_entry(mime) else {
            continue;
        };
        if let Some(command) = exec_from_desktop_file(&entry) {
            tracing::debug!(%mime, entry, program = command.program, "IDE resolved from desktop defaults");
            return Some(command);
        }
    }
    None
}

/// Asks `xdg-mime` which `.desktop` handles `mime`.
fn default_desktop_entry(mime: &str) -> Option<String> {
    let output = std::process::Command::new("xdg-mime")
        .args(["query", "default", mime])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// Reads the `Exec` line out of a `.desktop` file, wherever it lives.
fn exec_from_desktop_file(entry: &str) -> Option<IdeCommand> {
    let contents = desktop_search_paths()
        .into_iter()
        .map(|dir| dir.join(entry))
        .find_map(|path| std::fs::read_to_string(path).ok())?;
    parse_exec(&contents)
}

/// Every directory the XDG spec says `.desktop` files live in.
fn desktop_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        paths.push(PathBuf::from(home).join("applications"));
    } else if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".local/share/applications"));
    }
    // Flatpak exports are not always in XDG_DATA_DIRS for a process that
    // did not come from a login shell, and they are where a Flatpak IDE
    // announces itself.
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".local/share/flatpak/exports/share/applications"));
    }
    paths.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));

    let dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
    paths.extend(
        dirs.split(':')
            .map(|dir| Path::new(dir).join("applications")),
    );
    paths
}

/// The `Exec` of the `[Desktop Entry]` section, with field codes removed.
fn parse_exec(contents: &str) -> Option<IdeCommand> {
    let mut in_entry = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some(exec) = line.strip_prefix("Exec=") else {
            continue;
        };
        // `%F`, `%U`, `%f`, `%u`, `%i`, `%c`, `%k` are placeholders the launcher substitutes. We
        // supply our own paths, so they go.
        let cleaned: Vec<String> = exec
            .split_whitespace()
            .filter(|part| !(part.len() == 2 && part.starts_with('%')))
            .map(|part| part.trim_matches(['"', '\'']).to_owned())
            .collect();
        return IdeCommand::parse(&cleaned.join(" "));
    }
    None
}

#[cfg(test)]
mod tests;
