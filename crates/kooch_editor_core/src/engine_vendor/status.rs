//! Reading the engine situation **without changing it**.
//!
//! [`ensure_current`](super::ensure_current) answers "make it right" and
//! does. This answers "what is it" and touches nothing, which is what a
//! panel needs to draw a choice — and what opening a project needs before
//! it decides whether there is a choice to offer at all.
//!
//! # Why opening a project stopped updating on its own
//!
//! It used to materialise this editor's engine over whatever was there,
//! and say so in a log line nobody reads. Two things are wrong with that:
//! the first rebuild afterwards is a full one, minutes long, with no
//! warning; and the project's `.so` was compiled against the engine that
//! just got replaced.
//!
//! ⚠️ **A missing engine is still materialised without asking.** There is
//! nothing to keep and nothing to lose, and a project that cannot build
//! is not a choice worth offering.
//!
//! # 🔴 The limit of naming engines by version alone
//!
//! One directory per `major.minor.patch`, replaced in place. So *Keep*
//! means "do not touch the disk now", and it holds for as long as nobody
//! updates from another project: two engines with the same version have
//! nowhere separate to live. Switching that would mean putting the tree
//! hash in the directory name, which was considered and turned down —
//! version numbers are what a person can reason about.

use std::path::{Path, PathBuf};

use super::stamp::EngineStamp;
use super::{editor_engine_version, is_engine_source, shared_engine_dir};

/// How the engine on disk relates to the one this editor would install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Difference {
    /// The installed engine came from this editor's source. Nothing to do.
    Current,
    /// Nothing installed for this version yet — a first run, or a first
    /// project after the editor moved to a new version.
    Absent,
    /// 🔴 Same version, different source tree. **The engine-development
    /// case**, and the one nothing used to report: `CARGO_PKG_VERSION`
    /// stays `0.1.0` across every build, so the version says the two are
    /// the same engine and the tree hash says they are not.
    Rebuilt,
    /// The project asks for a version this editor does not ship, and that
    /// version is already on the machine. Left alone on purpose: writing
    /// this editor's source into a directory named after another version
    /// puts an engine on disk under a name that is not its own (#761).
    OtherVersion,
    /// No source to install from and nothing installed. Not an error by
    /// itself — a project pointing at a good engine elsewhere still
    /// builds.
    NoSource,
}

impl Difference {
    /// Whether this is something to put in front of a person.
    ///
    /// [`Current`](Self::Current) is not news, and
    /// [`Absent`](Self::Absent) is handled without asking.
    pub fn wants_a_decision(&self) -> bool {
        matches!(self, Self::Rebuilt | Self::OtherVersion)
    }
}

/// What the editor found when it looked, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineStatus {
    /// Version the project's manifest asks for.
    pub project_version: String,
    /// Version this editor would install.
    pub editor_version: String,
    /// Where the engine the project would build against is, when there
    /// is one.
    pub installed: Option<PathBuf>,
    pub difference: Difference,
}

impl EngineStatus {
    /// The version Install materialises.
    ///
    /// 🔴 **Always this editor's, never the project's.**
    /// [`ensure_current`](super::ensure_current) honours a project's own
    /// version when that engine is already on the machine — deliberately,
    /// so a project pinned to an older engine keeps building, and so this
    /// editor never writes its source under a name that is not its own
    /// (#761).
    ///
    /// Install asked for exactly that version. With 0.1.0 on disk and the
    /// editor shipping 0.2.0, the call returned `UpToDate` pointing at the
    /// old directory: nothing installed, nothing reported, and the prompt
    /// straight back on the next frame.
    ///
    /// Installing moves the project onto this editor's version, which is
    /// what the prompt has always said it does.
    pub fn version_to_install(&self) -> &str {
        &self.editor_version
    }

    /// One line for a panel, saying which of the two questions this is.
    ///
    /// 🔴 "differs" on its own is the unhelpful version. With one
    /// directory per version, the same number on both sides *and* a
    /// difference is the normal state while developing the engine, and a
    /// person needs to be told which of the two situations they are in.
    pub fn headline(&self) -> String {
        match self.difference {
            Difference::Current => format!("Engine {} — up to date", self.project_version),
            Difference::Absent => format!("Engine {} is not installed yet", self.project_version),
            Difference::Rebuilt => format!(
                "Engine {} — same version, different source than this editor ships",
                self.project_version,
            ),
            Difference::OtherVersion => format!(
                "This project uses engine {}; this editor ships {}",
                self.project_version, self.editor_version,
            ),
            Difference::NoSource => {
                "No engine source to install from, and none installed".to_owned()
            }
        }
    }
}

/// Looks at the engine a project would build against, and changes
/// nothing.
///
/// Mirrors [`ensure_current`](super::ensure_current)'s reasoning exactly,
/// which is why the two live next to each other: a status that disagreed
/// with what applying would do is worse than no status at all.
pub fn status(project_version: &str, source: Option<&Path>) -> EngineStatus {
    let editor_version = editor_engine_version().to_owned();
    let of = |installed, difference| EngineStatus {
        project_version: project_version.to_owned(),
        editor_version: editor_version.clone(),
        installed,
        difference,
    };

    // A version this editor cannot supply, already on the machine: it is
    // what the project builds against and this editor has no business
    // replacing it.
    if project_version != editor_version
        && let Some(existing) = shared_engine_dir(project_version).filter(|d| is_engine_source(d))
    {
        return of(Some(existing), Difference::OtherVersion);
    }

    let Some(dest) = shared_engine_dir(&editor_version) else {
        return of(None, Difference::NoSource);
    };
    let difference = status_in(&dest, source);
    let installed = match difference {
        Difference::Absent | Difference::NoSource => None,
        _ => Some(dest),
    };
    of(installed, difference)
}

/// [`status`] against an explicit directory.
///
/// The half that does not need to know about versions, which is also the
/// half a test can drive without reaching for the real data directory.
pub fn status_in(dest: &Path, source: Option<&Path>) -> Difference {
    let present = is_engine_source(dest);

    let Some(source) = source.filter(|s| is_engine_source(s)) else {
        // Nothing to compare against. What is there is all there is, and
        // it builds.
        return match present {
            true => Difference::Current,
            false => Difference::NoSource,
        };
    };

    if !present {
        return Difference::Absent;
    }

    // 🔴 Identity, not shape. `is_engine_source` is true of every copy of
    // the engine ever made, including one from an editor three weeks old.
    match EngineStamp::of_source(source) {
        Ok(stamp) if EngineStamp::read(dest).as_ref() == Some(&stamp) => Difference::Current,
        Ok(_) => Difference::Rebuilt,
        // Unreadable source is the same situation as no source: nothing
        // to compare, and what is installed still builds.
        Err(_) => Difference::Current,
    }
}

/// One engine this machine has on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// `major.minor.patch`, read off the directory name.
    pub version: String,
    pub path: PathBuf,
}

/// Every engine installed on this machine, oldest name first.
///
/// Reads the directory rather than a registry, because the directory *is*
/// the registry: `ensure_current` creates one per version and nothing
/// else records them. A folder that does not hold engine source is
/// skipped rather than listed — a half-finished install (`engine.partial`)
/// is not something to offer anyone.
pub fn installed_engines() -> Vec<Installed> {
    let Some(any) = shared_engine_dir("x") else {
        return Vec::new();
    };
    // `<base>/x/engine` → `<base>`. Built from the same function that
    // resolves the real thing so an override like `KOOCH_ENGINE_HOME`
    // cannot be honoured in one place and missed in the other.
    let Some(base) = any.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };

    let mut found: Vec<Installed> = entries
        .flatten()
        .filter_map(|entry| {
            let version = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path().join(super::VENDOR_DIR);
            is_engine_source(&path).then_some(Installed { version, path })
        })
        .collect();
    found.sort_by(|a, b| a.version.cmp(&b.version));
    found
}

/// Deletes an installed engine.
///
/// ⚠️ Refuses the version this editor ships: it is the one every project
/// that opens next will be pointed at, so deleting it means the next
/// open re-copies it — work with no result, and a moment where a project
/// points at a directory that is not there.
pub fn remove_engine(version: &str) -> Result<(), super::VendorError> {
    if version == editor_engine_version() {
        return Ok(());
    }
    let Some(dir) = shared_engine_dir(version).and_then(|d| d.parent().map(Path::to_path_buf))
    else {
        return Ok(());
    };
    std::fs::remove_dir_all(dir).map_err(super::VendorError::Io)
}

#[cfg(test)]
mod tests;
