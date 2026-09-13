//! Copying the engine's source into a project (#754, phase 1).

mod copy;
pub mod stamp;
mod status;

pub use status::{Difference, EngineStatus, Installed, installed_engines, remove_engine, status};

use std::fs;
use std::path::{Path, PathBuf};

use copy::copy_engine_into;
use stamp::EngineStamp;

/// Directory name the materialised engine occupies, under the
/// per-version directory.
pub const VENDOR_DIR: &str = "engine";

/// Where this machine keeps the engine for `version`.
pub fn shared_engine_dir(version: &str) -> Option<PathBuf> {
    // `KOOCH_ENGINE_HOME` overrides the base. It exists for CI and for a portable install that must
    // not write to the user's data directory — and it is what lets this be tested without a test
    // writing into somebody's real ~/.local/share.
    let base = match std::env::var_os("KOOCH_ENGINE_HOME") {
        Some(dir) => Some(PathBuf::from(dir)),
        // 🔴 A test must never reach the real data directory. One did, and left a 12 KB fixture at
        // ~/.local/share/kooch/0.1.0/engine — which `is_engine_source` accepts, so the editor would
        // have reported it up to date and never materialised the real engine.
        None if cfg!(test) => None,
        None => dirs::data_dir().map(|d| d.join("kooch")),
    };
    base.map(|b| b.join(version).join(VENDOR_DIR))
}

/// Where this machine keeps a vendor SDK, under the same base the engine uses.
pub fn shared_sdk_dir(name: &str, version: &str) -> Option<PathBuf> {
    let base = match std::env::var_os("KOOCH_ENGINE_HOME") {
        Some(dir) => Some(PathBuf::from(dir)),
        None if cfg!(test) => None,
        None => dirs::data_dir().map(|d| d.join("kooch")),
    };
    base.map(|b| b.join("sdk").join(name).join(version))
}

/// What the engine's own root has to contain for a copy of it to be buildable. Checked before
/// writing anything, so a bad engine root fails project creation rather than producing a project
/// that cannot compile and does not say why.
const REQUIRED: [&str; 3] = ["Cargo.toml", "crates", "src"];

/// Top-level entries copied into the vendored engine.
const COPY: [&str; 6] = [
    "Cargo.toml",
    "Cargo.lock",
    "crates",
    "src",
    "templates",
    // 🔴 Mandatory. The facade does `include_str!("../LICENSE.md")`, so a materialised engine
    // without it does not compile at all — which is the point: the licence cannot be dropped from a
    // build by leaving a file behind.
    "LICENSE.md",
];

/// Engine assets a *game* needs at runtime.
pub(crate) const COPY_ASSETS: [&str; 2] = ["materials", "meshes"];

#[derive(Debug)]
pub enum VendorError {
    /// The engine root does not look like the engine.
    NotAnEngineRoot(PathBuf),
    Io(std::io::Error),
}

impl std::fmt::Display for VendorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnEngineRoot(p) => write!(
                f,
                "{} does not look like the engine source (expected {})",
                p.display(),
                REQUIRED.join(", "),
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for VendorError {}

/// `true` when `root` holds the engine's source rather than, say, an
/// install directory holding only a binary.
pub fn is_engine_source(root: &Path) -> bool {
    REQUIRED.iter().all(|entry| root.join(entry).exists())
}

/// Where this editor's copy of the engine source lives, or `None` when it has none to give.
pub fn vendor_source(engine_root: Option<&Path>) -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("KOOCH_ENGINE_SOURCE") {
        let path = PathBuf::from(explicit);
        if is_engine_source(&path) {
            return Some(path);
        }
        tracing::warn!(
            path = %path.display(),
            "KOOCH_ENGINE_SOURCE is set but does not look like engine source; ignoring",
        );
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let beside = dir.join(VENDOR_DIR);
        if is_engine_source(&beside) {
            return Some(beside);
        }
    }

    engine_root
        .filter(|root| is_engine_source(root))
        .map(Path::to_path_buf)
}

/// `true` when the editor is running out of the engine's own build directory — i.e. someone is
/// developing the engine, not using it.
pub fn running_from_engine_build(engine_root: &Path) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let target = engine_root.join("target");
    // Canonicalize both or neither: on macOS the exe path is a symlink
    // resolution away from the source tree and the prefix test fails.
    let (Ok(exe), Ok(target)) = (exe.canonicalize(), target.canonicalize()) else {
        return exe.starts_with(&target);
    };
    exe.starts_with(target)
}

/// Copies the engine's source into `<project_root>/engine/`.
pub fn vendor_engine(project_root: &Path, source: &Path) -> Result<PathBuf, VendorError> {
    // Checked before the directory is created, not after: a refused
    // vendor must leave nothing behind, or the next run finds a stub
    // and the failure changes shape.
    if !is_engine_source(source) {
        return Err(VendorError::NotAnEngineRoot(source.to_path_buf()));
    }
    let dest = project_root.join(VENDOR_DIR);
    fs::create_dir_all(&dest).map_err(VendorError::Io)?;
    copy_engine_into(&dest, source)?;
    // The copy says which tree it came from. `package_editor` calls this
    // to lay out a distributable editor, so this is where the stamp an
    // installed editor later propagates is first written (#761).
    EngineStamp::of_source(source)?.write(&dest)?;
    Ok(dest)
}

/// The engine version this editor would vendor.
pub fn editor_engine_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// What [`ensure_current`] had to do, for the caller to log.
#[derive(Debug, PartialEq, Eq)]
pub enum VendorState {
    /// Already materialised, from this exact source.
    UpToDate,
    /// Was not on this machine yet — a first run, or a first project
    /// after the editor updated.
    Materialised,
    /// Was there, from a different source tree, and has been replaced (#761).
    Replaced,
    /// No engine source to materialise from, and none already there.
    /// Not an error on its own: a project pointing at a good copy still
    /// builds.
    NoSourceAvailable,
}

/// Makes this machine's copy of the engine exist, and says where it is.
pub fn ensure_current(
    wanted: &str,
    source: Option<&Path>,
) -> Result<(VendorState, Option<PathBuf>), VendorError> {
    // 🔴 The version a project asks for and the version this editor can supply are different
    // questions, and conflating them writes a lie to disk: materialising THIS editor's source into
    // a directory named after the project's version.
    let own =
        shared_engine_dir(editor_engine_version()).map(|dest| ensure_current_in(&dest, source));

    if wanted != editor_engine_version()
        && let Some(existing) = shared_engine_dir(wanted).filter(|d| is_engine_source(d))
    {
        return Ok((VendorState::UpToDate, Some(existing)));
    }
    // The return still describes the engine the PROJECT got, which is
    // this editor's whenever the branch above did not fire.
    own.unwrap_or(Ok((VendorState::NoSourceAvailable, None)))
}

/// [`ensure_current`] against an explicit directory.
pub fn ensure_current_in(
    dest: &Path,
    source: Option<&Path>,
) -> Result<(VendorState, Option<PathBuf>), VendorError> {
    let dest = dest.to_path_buf();
    let present = is_engine_source(&dest);

    let Some(source) = source.filter(|s| is_engine_source(s)) else {
        // Nothing to materialise from, and nothing to compare against.
        // What is already there is all there is, and it builds.
        return Ok(match present {
            true => (VendorState::UpToDate, Some(dest)),
            false => (VendorState::NoSourceAvailable, None),
        });
    };

    // 🔴 Identity, not shape. `is_engine_source` is true of every copy of the engine ever made,
    // including one from an editor three weeks old — which is how a new install went on compiling
    // projects against stale source in silence (#761).
    let stamp = EngineStamp::of_source(source)?;
    if present && EngineStamp::read(&dest).as_ref() == Some(&stamp) && !damaged(&dest) {
        return Ok((VendorState::UpToDate, Some(dest)));
    }

    materialise(&dest, source, &stamp)?;
    Ok((
        match present {
            true => VendorState::Replaced,
            false => VendorState::Materialised,
        },
        Some(dest),
    ))
}

/// Whether `dest` no longer holds what its own stamp says, so it should be replaced even though the
/// source has not changed.
fn damaged(dest: &Path) -> bool {
    if std::env::var_os("KOOCH_VERIFY_ENGINE").is_none() {
        return false;
    }
    match EngineStamp::check(dest) {
        Ok(stamp::Check::Match) => false,
        Ok(stamp::Check::Differs { recorded, actual }) => {
            tracing::warn!(
                path = %dest.display(),
                recorded = format!("{recorded:016x}"),
                actual = format!("{actual:016x}"),
                "the vendored engine is not what it records — re-materialising",
            );
            true
        }
        // Unstamped is not damage: the stamp comparison above already
        // treats it as stale and is about to replace the directory.
        Ok(stamp::Check::NoStamp) => false,
        // A tree that cannot be read is a tree that cannot be trusted,
        // and re-copying is the repair.
        Err(e) => {
            tracing::warn!(
                path = %dest.display(),
                error = %e,
                "the vendored engine could not be verified",
            );
            true
        }
    }
}

/// Puts `source` at `dest`, replacing whatever was there, leaving one copy behind and never a
/// half-written one.
fn materialise(dest: &Path, source: &Path, stamp: &EngineStamp) -> Result<(), VendorError> {
    let staging = dest.with_extension("partial");
    let stale = dest.with_extension("stale");
    // Leftovers from a run that died mid-swap. Both renames below fail if
    // their target exists, so this is repair, not tidying.
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_dir_all(&stale);
    if let Some(parent) = staging.parent() {
        fs::create_dir_all(parent).map_err(VendorError::Io)?;
    }
    fs::create_dir_all(&staging).map_err(VendorError::Io)?;
    copy_engine_into(&staging, source)?;
    stamp.write(&staging)?;

    let had_old = dest.exists();
    if had_old {
        fs::rename(dest, &stale).map_err(VendorError::Io)?;
    }
    if let Err(e) = fs::rename(&staging, dest) {
        // Put back what was working. Failing with the old engine still in
        // place is a bad update; failing with no engine at all is a
        // machine that cannot build anything.
        if had_old {
            let _ = fs::rename(&stale, dest);
        }
        return Err(VendorError::Io(e));
    }
    if had_old {
        let _ = fs::remove_dir_all(&stale);
    }
    Ok(())
}

/// Taken by every test that sets `KOOCH_ENGINE_HOME`.
#[cfg(test)]
pub(crate) static ENGINE_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests;

#[cfg(test)]
mod stamp_tests;

#[cfg(test)]
mod reach_tests;
