//! [`SceneRoot`] — the one folder a remote scene read or write may touch (#647).

use std::path::{Path, PathBuf};

use kooch_core::resource::Resources;

use crate::protocol::RemoteError;

/// The project folder. Anything that reaches the socket gets a file read or write, so the paths it
/// names are held inside this; the launcher passes it as `KOOCH_PROJECT_ROOT`.
#[derive(Debug, Clone)]
pub struct SceneRoot(pub PathBuf);

impl SceneRoot {
    /// `KOOCH_PROJECT_ROOT`, else the working directory the editor launched us in.
    pub fn from_env() -> Self {
        let root = std::env::var_os("KOOCH_PROJECT_ROOT")
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        Self(root)
    }
}

/// Resolves `path` against the world's [`SceneRoot`], refusing anything that lands outside it.
pub(crate) fn confined(resources: &Resources, path: &str) -> Result<PathBuf, RemoteError> {
    let root = match resources.get::<SceneRoot>() {
        Some(root) => root.clone(),
        None => SceneRoot::from_env(),
    };
    confine(&root.0, Path::new(path)).map_err(|detail| RemoteError::BadRequest { detail })
}

/// `path` inside `root`, or why not. Symlinks are resolved, so a link out of the project is out.
pub(crate) fn confine(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|e| {
        format!(
            "the project root {} cannot be resolved: {e}",
            root.display()
        )
    })?;
    let wanted = root.join(path);

    // A scene not saved yet has no canonical form: resolve the deepest folder that exists and
    // append the rest. `file_name` is never `..`, so the appended part cannot climb out.
    let mut existing = wanted.as_path();
    let mut rest = Vec::new();
    let resolved = loop {
        if let Ok(found) = existing.canonicalize() {
            break found;
        }
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            return Err(format!("{} cannot be resolved", path.display()));
        };
        rest.push(name.to_owned());
        existing = parent;
    };
    let resolved = rest.iter().rev().fold(resolved, |acc, name| acc.join(name));

    match resolved.starts_with(&root) {
        true => Ok(resolved),
        false => Err(format!("{} is outside the project", path.display())),
    }
}

#[cfg(test)]
mod tests;
