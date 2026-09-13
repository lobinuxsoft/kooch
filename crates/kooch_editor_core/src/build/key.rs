//! Where a project keeps the key its packs are sealed with (#758).

use std::path::{Path, PathBuf};

use kooch_pack::PackKey;

/// Directory a project keeps editor-owned local state in.
pub const LOCAL_DIR: &str = ".kooch";

/// File the pack key lives in, under [`LOCAL_DIR`].
pub const KEY_FILE: &str = "pack.key";

/// Environment variable that overrides the file.
pub const KEY_ENV: &str = "KOOCH_PACK_KEY";

/// Reads the project's pack key, generating and saving one the first time.
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

#[cfg(test)]
/// Whether this project already has a key on disk.
pub fn has_key(project_root: &Path) -> bool {
    key_path(project_root).is_file()
}

/// Where the key file lives for a project.
pub fn key_path(project_root: &Path) -> PathBuf {
    project_root.join(LOCAL_DIR).join(KEY_FILE)
}

/// Makes the key readable by its owner only.
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
