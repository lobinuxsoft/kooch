//! Copying the engine's source into a project (#754, phase 1).
//!
//! A project compiles the engine — the gameplay is native Rust and links
//! it as an `rlib`. Until now the generated `Cargo.toml` pointed at
//! whatever absolute path the engine happened to live at on the machine
//! that created the project, which meant a project was not portable
//! between two clones, let alone to someone holding only a compiled
//! editor.
//!
//! So the editor materialises the engine **once per version**, in
//! `~/.local/share/kooch/<version>/engine`, and every project's manifest
//! points at that. One copy on the machine, none inside any project.
//!
//! # Why not a copy per project
//!
//! It was, for one afternoon, and seeing it settled the question: 675
//! files of engine source inside a game's own directory is not what any
//! engine does, and duplicating them per project compounds it. The
//! editor owns this directory the way cargo owns `target/`.
//!
//! # Why the source is on disk at all
//!
//! Because Rust has no stable ABI. A precompiled `rlib` only links
//! against the exact compiler and the exact dependency versions that
//! built it, and cargo does not model binary dependencies at all — which
//! is why no Rust engine ships binaries, Bevy included. The only route
//! to "binary, no source" is an `extern "C"` API in the shape of
//! GDExtension, and that costs the typed ECS.
//!
//! So the engine's source is protected the way Unreal protects theirs:
//! **by licence, not by hiding it**. The repo is public and ARR already.
//!
//! ⚠️ The manifest ends up holding an absolute path, and `$HOME` differs
//! per user. [`ensure_current`] rewrites that line when it does not
//! match this machine — the editor owns it, the same way it owns the
//! directory it points at.
//!
//! # Why source and not a binary
//!
//! Godot ships its engine as a precompiled library the project loads,
//! and that is the model this is imitating. It is not the model this can
//! *implement*: Rust has no stable ABI, so the engine and the game have
//! to come out of the same compiler. Vendoring the source gets the same
//! result — a self-contained project — and pays for it in build time
//! rather than in download size.
//!
//! # What it costs, measured
//!
//! About 8 MB of text. **No build time at all**: a project already
//! compiled the engine from source, it just reached outside its own
//! directory to find it. The `target/` of an existing project is
//! gigabytes for exactly that reason.

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
///
/// One directory per version rather than one overall: two projects
/// pinned to different engine versions have to coexist, and blowing one
/// away when the other opens would make switching projects a rebuild.
///
/// `None` only when the platform has no data directory, which is the
/// case a caller has to handle rather than unwrap.
pub fn shared_engine_dir(version: &str) -> Option<PathBuf> {
    // `KOOCH_ENGINE_HOME` overrides the base. It exists for CI and for
    // a portable install that must not write to the user's data
    // directory — and it is what lets this be tested without a test
    // writing into somebody's real ~/.local/share.
    let base = match std::env::var_os("KOOCH_ENGINE_HOME") {
        Some(dir) => Some(PathBuf::from(dir)),
        // 🔴 A test must never reach the real data directory. One did,
        // and left a 12 KB fixture at ~/.local/share/kooch/0.1.0/engine
        // — which `is_engine_source` accepts, so the editor would have
        // reported it up to date and never materialised the real engine.
        // Every project on the machine would have pointed at a stub.
        None if cfg!(test) => None,
        None => dirs::data_dir().map(|d| d.join("kooch")),
    };
    base.map(|b| b.join(version).join(VENDOR_DIR))
}

/// What the engine's own root has to contain for a copy of it to be
/// buildable. Checked before writing anything, so a bad engine root
/// fails project creation rather than producing a project that cannot
/// compile and does not say why.
const REQUIRED: [&str; 3] = ["Cargo.toml", "crates", "src"];

/// Top-level entries copied into the vendored engine.
///
/// An allowlist rather than a denylist, deliberately: a denylist admits
/// whatever the engine repo grows next, and the first symptom of getting
/// it wrong is every new project carrying a few hundred megabytes of
/// somebody's `target/`.
///
/// 🔴 The failure mode of an allowlist is the opposite one — omitting
/// something the build needs — and it costs a full compile to find.
/// `templates/` was missed exactly that way: `kooch_editor_core` reaches
/// it with `include_str!("../../../../templates/…")`, so the crate
/// compiles inside the engine repo and not inside a vendored copy.
/// `reach_tests::vendored_includes_all_resolve` scans for that pattern so
/// the next one is a test failure instead of a ten-minute build.
const COPY: [&str; 6] = [
    "Cargo.toml",
    "Cargo.lock",
    "crates",
    "src",
    "templates",
    // 🔴 Mandatory. The facade does `include_str!("../LICENSE.md")`, so
    // a materialised engine without it does not compile at all — which
    // is the point: the licence cannot be dropped from a build by
    // leaving a file behind.
    "LICENSE.md",
];

/// Engine assets a *game* needs at runtime. The engine's `assets/` is
/// 13 MB, and 12.7 MB of that is two demo glTFs that no shipped game
/// loads — so this takes the two directories that are actually
/// referenced and leaves the samples behind.
pub(crate) const COPY_ASSETS: [&str; 2] = ["materials", "meshes/primitives"];

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

/// Where this editor's copy of the engine source lives, or `None` when
/// it has none to give.
///
/// Resolution order, and each entry is there for a case that happens:
///
/// 1. **`KOOCH_ENGINE_SOURCE`** — an explicit override. What CI uses,
///    and the escape hatch when the layout below is not what someone
///    built.
/// 2. **`<dir of the executable>/engine/`** — the install layout. This
///    is the whole of #754 phase 2: a compiled editor ships the source
///    beside itself, so it can hand a project a working engine without
///    a clone anywhere on the machine.
/// 3. **The engine root** — running from the engine's own tree, i.e.
///    developing the engine.
///
/// Returns `None` rather than guessing. A wrong directory here produces
/// a project that fails to build with an error naming a missing crate,
/// which says nothing about vendoring.
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

/// `true` when the editor is running out of the engine's own build
/// directory — i.e. someone is developing the engine, not using it.
///
/// 🔴 "Is `engine_root` engine source?" does NOT answer this, which was
/// the first attempt. Once a compiled editor ships the source alongside
/// itself (#754 phase 2) that is true for every install, and no project
/// would ever get its own copy.
///
/// The signal that does separate them is where the **executable** is:
/// `cargo run` puts it in `<engine>/target/…`, an installed editor puts
/// it anywhere else. Vendoring in the first case would freeze a project
/// against a snapshot and break the daily loop of changing engine and
/// game together.
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
///
/// Returns the path written to. Refuses a `source` that is not an engine
/// root, because the alternative is a project whose `cargo build` fails
/// on a missing crate and gives no hint that the copy was the problem.
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
    /// Was there, from a different source tree, and has been replaced
    /// (#761).
    ///
    /// Distinct from [`Materialised`](Self::Materialised) because it is
    /// the interesting one to read in a log: it is the moment a project
    /// stops building against the engine it built against yesterday, and
    /// the first rebuild after it is a full one.
    Replaced,
    /// No engine source to materialise from, and none already there.
    /// Not an error on its own: a project pointing at a good copy still
    /// builds.
    NoSourceAvailable,
}

/// Makes this machine's copy of the engine exist, and says where it is.
///
/// One directory per engine version, shared by every project. Called
/// when a project opens and when one is created: the first is the case
/// of a machine that has never seen this version, the second of a
/// project that does not exist yet.
///
/// Never deletes another *version*. Two versions coexist; the old one
/// stays for the projects still pinned to it. A directory holding a
/// stale copy of *this* version is replaced — see [`ensure_current_in`].
pub fn ensure_current(
    wanted: &str,
    source: Option<&Path>,
) -> Result<(VendorState, Option<PathBuf>), VendorError> {
    // 🔴 The version a project asks for and the version this editor can
    // supply are different questions, and conflating them writes a lie
    // to disk: materialising THIS editor's source into a directory named
    // after the project's version. The `.so` would then load against an
    // engine that is not what its directory claims, and the BuildStamp
    // would only catch it later, from somewhere else.
    //
    // So: honour the project's version when that engine is already on
    // the machine, and otherwise give it the only one available — this
    // editor's — under its own honest name. The caller records the
    // change in the manifest.
    //
    // 🔴 The stamp check below applies only to THIS editor's directory.
    // A version the machine already has and this editor does not ship is
    // left exactly as it is: the source in hand is not what that
    // directory is supposed to hold, so "it differs" is not a reason to
    // overwrite it — it is the reason it exists (#761).
    if wanted != editor_engine_version()
        && let Some(existing) = shared_engine_dir(wanted).filter(|d| is_engine_source(d))
    {
        return Ok((VendorState::UpToDate, Some(existing)));
    }
    let Some(dest) = shared_engine_dir(editor_engine_version()) else {
        return Ok((VendorState::NoSourceAvailable, None));
    };
    ensure_current_in(&dest, source)
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

    // 🔴 Identity, not shape. `is_engine_source` is true of every copy of
    // the engine ever made, including one from an editor three weeks old
    // — which is how a new install went on compiling projects against
    // stale source in silence (#761).
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

/// Whether `dest` no longer holds what its own stamp says, so it should
/// be replaced even though the source has not changed.
///
/// **Off unless `KOOCH_VERIFY_ENGINE` is set**, because it reads the
/// whole 8 MB tree and this runs every time a project opens. The stamp
/// comparison above catches a stale engine; it cannot catch a damaged
/// one, since deleting a file from a copy does not alter what the copy
/// claims to be.
///
/// Reports rather than returns detail: the only decision downstream is
/// whether to re-copy, and everything worth reading — which hash, which
/// directory — belongs in a log rather than in a bool's type.
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

/// Puts `source` at `dest`, replacing whatever was there, leaving one
/// copy behind and never a half-written one.
///
/// # The order, and why it is that order
///
/// A half-written directory from an interrupted copy would pass
/// [`is_engine_source`] on the next run and never be repaired, so the
/// copy lands beside its destination and is renamed in — atomic within
/// one filesystem.
///
/// The old copy is moved aside *before* the new one takes its place
/// rather than deleted: `rename` onto a non-empty directory fails, and
/// deleting first would leave the machine with no engine at all if the
/// copy then failed. It is removed once the new one is in place, so a
/// replacement does not leave two trees on disk — the whole point being
/// that the same source is not stored twice.
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
///
/// 🔴 The environment belongs to the **process**, and cargo's harness is
/// threaded. A test that sets the variable changes where every other
/// test's `shared_engine_dir` points, including tests that never touch
/// it — they are the ones that lose the race, and the failure is
/// intermittent and reads like a bug in the vendoring.
///
/// The comment these tests carried said "single-threaded by
/// `--test-threads=1`", which is not something a test file can assert
/// about the harness running it. `KOOCH_PACK_KEY` had the identical
/// comment and the identical race, and it was found by an unrelated test
/// failing intermittently.
#[cfg(test)]
pub(crate) static ENGINE_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests;

#[cfg(test)]
mod stamp_tests;

#[cfg(test)]
mod reach_tests;
