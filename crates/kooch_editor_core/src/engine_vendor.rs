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

use std::fs;
use std::path::{Path, PathBuf};

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
/// `every_directory_the_source_reaches_for_is_vendored` scans for that
/// pattern so the next one is a test failure instead of a ten-minute
/// build.
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
const COPY_ASSETS: [&str; 2] = ["materials", "meshes/primitives"];

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
    Ok(dest)
}

/// Copies the engine's source into `dest`, which must already exist.
fn copy_engine_into(dest: &Path, source: &Path) -> Result<(), VendorError> {
    if !is_engine_source(source) {
        return Err(VendorError::NotAnEngineRoot(source.to_path_buf()));
    }
    for entry in COPY {
        let from = source.join(entry);
        if from.is_dir() {
            copy_dir(&from, &dest.join(entry))?;
        } else if from.is_file() {
            fs::copy(&from, dest.join(entry)).map_err(VendorError::Io)?;
        }
    }
    for entry in COPY_ASSETS {
        let from = source.join("assets").join(entry);
        if from.is_dir() {
            copy_dir(&from, &dest.join("assets").join(entry))?;
        }
    }
    Ok(())
}

/// The engine version this editor would vendor.
pub fn editor_engine_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// What [`ensure_current`] had to do, for the caller to log.
#[derive(Debug, PartialEq, Eq)]
pub enum VendorState {
    /// Already materialised for this version.
    UpToDate,
    /// Was not on this machine yet — a first run, or a first project
    /// after the editor updated.
    Materialised,
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
/// Never deletes an existing copy. Two versions coexist; the old one
/// stays for the projects still pinned to it.
pub fn ensure_current(
    version: &str,
    source: Option<&Path>,
) -> Result<(VendorState, Option<PathBuf>), VendorError> {
    let Some(dest) = shared_engine_dir(version) else {
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
    if is_engine_source(&dest) {
        return Ok((VendorState::UpToDate, Some(dest)));
    }

    let Some(source) = source.filter(|s| is_engine_source(s)) else {
        return Ok((VendorState::NoSourceAvailable, None));
    };

    // A half-written directory from an interrupted copy would pass
    // `is_engine_source` on the next run and never be repaired, so the
    // copy lands beside its destination and is moved into place at the
    // end. `rename` within one filesystem is atomic.
    let staging = dest.with_extension("partial");
    let _ = fs::remove_dir_all(&staging);
    if let Some(parent) = staging.parent() {
        fs::create_dir_all(parent).map_err(VendorError::Io)?;
    }
    fs::create_dir_all(&staging).map_err(VendorError::Io)?;
    copy_engine_into(&staging, source)?;
    fs::rename(&staging, &dest).map_err(VendorError::Io)?;

    Ok((VendorState::Materialised, Some(dest)))
}

/// `true` for a path that is test code rather than engine code.
///
/// The engine's tests live in their own files — `#[cfg(test)] mod X;`
/// beside the module, never a block inside it — so keeping them out of
/// a project is a matter of not copying files rather than of stripping
/// code out of them. `#[cfg(test)]` removes the `mod` before Rust
/// resolves it, so the engine compiles with none of these present.
/// Verified, not assumed: see the module docs.
///
/// `cfg_test_mods` carries the names the enclosing directory's own
/// sources declared under `#[cfg(test)]`. 🔴 Reading the declaration
/// rather than matching a filename is the difference between a rule and
/// a convention: three of the engine's test files are called
/// `measure.rs` and `id_stability.rs`, and a name-based filter shipped
/// all three.
fn is_test_code(path: &Path, cfg_test_mods: &[String]) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if path.is_dir() {
        return name == "tests" || name == "benches" || cfg_test_mods.iter().any(|m| m == name);
    }
    let stem = name.strip_suffix(".rs").unwrap_or("");
    name == "tests.rs" || name.ends_with("_tests.rs") || cfg_test_mods.iter().any(|m| m == stem)
}

/// Module names this directory's own `.rs` files declare under
/// `#[cfg(test)]`, i.e. the files that exist only for tests.
fn cfg_test_modules(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();

    // 🔴 For `foo/`, the declarations live in `foo.rs` — one level UP,
    // not inside. Rust puts a module's submodules in a directory named
    // after it while the `mod` lines stay in the file. Missing this
    // shipped `sys_metrics/measure.rs`, which is nothing but a
    // `#[cfg(test)]` benchmark.
    let sibling = dir.with_extension("rs");
    if let Ok(text) = fs::read_to_string(&sibling) {
        names.extend(cfg_test_mods_in(&text));
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            names.extend(cfg_test_mods_in(&text));
        }
    }
    names
}

/// Module names one source file declares under `#[cfg(test)]`.
fn cfg_test_mods_in(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut gated = false;
    for line in text.lines() {
        let line = line.trim();
        if line == "#[cfg(test)]" {
            gated = true;
            continue;
        }
        if gated && let Some(name) = declared_module(line) {
            names.push(name);
        }
        // Attributes between the gate and the item keep the gate;
        // anything else ends it.
        if !line.starts_with('#') && !line.is_empty() {
            gated = false;
        }
    }
    names
}

/// The name in `mod X;` / `pub(crate) mod X;`, if the line is one.
fn declared_module(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub "))
        .unwrap_or(line);
    rest.strip_prefix("mod ")?
        .strip_suffix(';')
        .map(str::to_owned)
}

/// Recursive copy, skipping build output.
///
/// `target/` is skipped at every level and not only the top: a workspace
/// member that was ever built standalone has one of its own, and a
/// single missed check is the difference between 8 MB and gigabytes.
fn copy_dir(from: &Path, to: &Path) -> Result<(), VendorError> {
    fs::create_dir_all(to).map_err(VendorError::Io)?;
    let test_mods = cfg_test_modules(from);
    for entry in fs::read_dir(from).map_err(VendorError::Io)? {
        let entry = entry.map_err(VendorError::Io)?;
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let src = entry.path();
        // 🔴 No test code leaves this repo. A project's copy of the
        // engine is for building a game, and the tests are the engine's
        // own business.
        if is_test_code(&src, &test_mods) {
            continue;
        }
        let dst = to.join(&name);
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            fs::copy(&src, &dst).map_err(VendorError::Io)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
