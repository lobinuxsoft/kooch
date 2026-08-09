//! Where a project keeps the key its packs are sealed with (#758).
//!
//! `<project>/.kooch/pack.key`, and **not in version control**. The
//! scaffold's `.gitignore` excludes `.kooch/`, so a project created by
//! this editor cannot commit one by accident.
//!
//! # Why not in the preset
//!
//! A preset is configuration: which target, which folder, packed or not.
//! It belongs in the repository so everyone builds the same thing. A key
//! does not — a repository carrying it has published it, and history
//! keeps it published after the file is deleted.
//!
//! Godot draws the same line, between `export_presets.cfg` and its
//! encryption key.
//!
//! # 🔴 One key per project, generated once
//!
//! Generated on first use and kept. Not regenerated per build, because
//! then the editor could not open yesterday's pack to check it, and not
//! shared between projects, because breaking one would break them all.
//!
//! And never a key belonging to the *editor*: a single extraction from
//! one published editor binary would open the packs of every game ever
//! made with it.
//!
//! # It can be overridden
//!
//! `KOOCH_PACK_KEY` wins when set. That is what a CI build uses, where
//! the key arrives from a secret store and no file should be written.

use std::path::{Path, PathBuf};

use kooch_pack::PackKey;

/// Directory a project keeps editor-owned local state in.
pub const LOCAL_DIR: &str = ".kooch";

/// File the pack key lives in, under [`LOCAL_DIR`].
pub const KEY_FILE: &str = "pack.key";

/// Environment variable that overrides the file.
pub const KEY_ENV: &str = "KOOCH_PACK_KEY";

/// Reads the project's pack key, generating and saving one the first
/// time.
///
/// Order: the environment, then the file, then a fresh key.
pub fn project_key(project_root: &Path) -> Result<PackKey, std::io::Error> {
    if let Some(text) = std::env::var_os(KEY_ENV) {
        return PackKey::parse(&text.to_string_lossy()).ok_or_else(|| {
            // Named as *that* variable rather than a parse error: a key
            // mangled in a CI secret produces packs nothing can open,
            // and the message has to point at where it came from.
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{KEY_ENV} is not 64 hex characters"),
            )
        });
    }

    let path = key_path(project_root);
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Some(key) = PackKey::parse(&text)
    {
        return Ok(key);
    }

    let key = PackKey::generate();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, key.to_hex())?;
    restrict(&path);
    // 🔴 The path, never the key. This line exists so somebody knows the
    // file appeared and where; a key in a log is a key in every bug
    // report that log is pasted into.
    tracing::info!(
        path = %path.display(),
        "generated this project's asset pack key — keep it out of version control, \
         and keep a copy: without it nobody can open the packs you ship",
    );
    Ok(key)
}

/// Whether this project already has a key on disk.
pub fn has_key(project_root: &Path) -> bool {
    key_path(project_root).is_file()
}

/// Where the key file lives for a project.
pub fn key_path(project_root: &Path) -> PathBuf {
    project_root.join(LOCAL_DIR).join(KEY_FILE)
}

/// Makes the key readable by its owner only.
///
/// Best effort, and deliberately not an error: a key that could not be
/// chmod'd is still a working key, and refusing to build over a file mode
/// would be worse than the exposure on a single-user machine. Windows has
/// no equivalent this cheap.
#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!(path = %path.display(), error = %e, "could not restrict the key file");
    }
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {}

#[cfg(test)]
mod key_tests;
