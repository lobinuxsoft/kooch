//! Copying the engine's source into a project (#754, phase 1).
//!
//! A project compiles the engine — the gameplay is native Rust and links
//! it as an `rlib`. Until now the generated `Cargo.toml` pointed at
//! whatever absolute path the engine happened to live at on the machine
//! that created the project, which meant a project was not portable
//! between two clones, let alone to someone holding only a compiled
//! editor.
//!
//! So the project carries the engine. `<project>/engine/` holds the
//! source and the manifest says `path = "engine"` — the same line on
//! every machine.
//!
//! # It is build output, not source
//!
//! The **editor** puts it there and the editor replaces it when it goes
//! stale, so a project's `engine/` is regenerated rather than authored —
//! the same category as `target/`, and gitignored for the same reason.
//! [`ensure_current`] is what makes that true: opening a project with a
//! missing or outdated copy re-materialises it before anything tries to
//! build.
//!
//! ⚠️ The consequence, worth knowing rather than discovering: a clone of
//! a game repo does **not** build without the editor materialising the
//! engine first, and a project compiles against the editor's version
//! rather than the one it was authored against.
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

/// Directory a vendored engine occupies inside a project.
pub const VENDOR_DIR: &str = "engine";

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
const COPY: [&str; 4] = ["Cargo.toml", "Cargo.lock", "crates", "src"];

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
    if !is_engine_source(source) {
        return Err(VendorError::NotAnEngineRoot(source.to_path_buf()));
    }
    let dest = project_root.join(VENDOR_DIR);
    fs::create_dir_all(&dest).map_err(VendorError::Io)?;

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

    Ok(dest)
}

/// The engine version this editor would vendor.
pub fn editor_engine_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// What [`ensure_current`] had to do, for the caller to log.
#[derive(Debug, PartialEq, Eq)]
pub enum VendorState {
    /// Present and at this editor's version.
    UpToDate,
    /// Was not there at all — a fresh clone of a game repo, where the
    /// engine is gitignored.
    Materialised,
    /// Was there at an older version and has been replaced.
    Replaced { was: String },
    /// The editor has no engine source to vendor from, so whatever the
    /// project already has stays. Not an error: a project with a good
    /// copy still builds.
    NoSourceAvailable,
}

/// Makes `<project_root>/engine/` exist and match this editor.
///
/// Called when a project opens. A missing copy is the normal case for a
/// freshly cloned game repo; an outdated one is the normal case after
/// the editor updates. Both end with the project building against the
/// engine the editor ships, which is the point of the editor owning
/// this directory.
///
/// `project_engine_version` is the manifest's `engine_version` — the
/// version the copy on disk was written at. It is the only record of
/// that: the copied source carries no version the editor can trust to
/// have stayed in sync.
pub fn ensure_current(
    project_root: &Path,
    project_engine_version: &str,
    source: Option<&Path>,
) -> Result<VendorState, VendorError> {
    let dest = project_root.join(VENDOR_DIR);
    let present = is_engine_source(&dest);
    let current = present && project_engine_version == editor_engine_version();
    if current {
        return Ok(VendorState::UpToDate);
    }

    let Some(source) = source.filter(|s| is_engine_source(s)) else {
        return Ok(VendorState::NoSourceAvailable);
    };

    if present {
        // Replaced wholesale rather than merged. A crate deleted from
        // the engine would otherwise survive in every project that ever
        // vendored it, and be compiled by the workspace glob.
        fs::remove_dir_all(&dest).map_err(VendorError::Io)?;
    }
    vendor_engine(project_root, source)?;

    Ok(match present {
        true => VendorState::Replaced {
            was: project_engine_version.to_owned(),
        },
        false => VendorState::Materialised,
    })
}

/// Recursive copy, skipping build output.
///
/// `target/` is skipped at every level and not only the top: a workspace
/// member that was ever built standalone has one of its own, and a
/// single missed check is the difference between 8 MB and gigabytes.
fn copy_dir(from: &Path, to: &Path) -> Result<(), VendorError> {
    fs::create_dir_all(to).map_err(VendorError::Io)?;
    for entry in fs::read_dir(from).map_err(VendorError::Io)? {
        let entry = entry.map_err(VendorError::Io)?;
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let src = entry.path();
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
mod tests {
    use super::*;

    /// Builds a directory that passes for an engine root, with a
    /// `target/` in it to be skipped.
    fn fake_engine(root: &Path) {
        fs::create_dir_all(root.join("crates/kooch_core/src")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("assets/materials")).unwrap();
        fs::create_dir_all(root.join("assets/meshes/primitives")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join("crates/kooch_core/target")).unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]").unwrap();
        fs::write(root.join("Cargo.lock"), "# lock").unwrap();
        fs::write(root.join("src/lib.rs"), "// facade").unwrap();
        fs::write(root.join("crates/kooch_core/src/lib.rs"), "// core").unwrap();
        fs::write(root.join("assets/materials/default.material"), "()").unwrap();
        fs::write(root.join("assets/meshes/primitives/cube.glb"), "glb").unwrap();
        fs::write(root.join("assets/meshes/demo.glb"), &vec![0u8; 4096]).unwrap();
        fs::write(root.join("target/debug/huge"), &vec![0u8; 65536]).unwrap();
        fs::write(
            root.join("crates/kooch_core/target/nested"),
            &vec![0u8; 65536],
        )
        .unwrap();
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kooch_vendor_{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_vendored_engine_has_what_a_build_needs() {
        let dir = tmp("copies");
        let (engine, project) = (dir.join("engine_src"), dir.join("proj"));
        fake_engine(&engine);
        fs::create_dir_all(&project).unwrap();

        let dest = vendor_engine(&project, &engine).expect("vendors");

        assert!(dest.join("Cargo.toml").is_file());
        assert!(dest.join("Cargo.lock").is_file());
        assert!(dest.join("src/lib.rs").is_file());
        assert!(dest.join("crates/kooch_core/src/lib.rs").is_file());
    }

    /// 🔴 The check that keeps this feature from being a disaster. A
    /// missed `target/` turns "8 MB of source" into a copy of somebody's
    /// entire build directory, and the nested one is the easy miss: a
    /// workspace member built standalone has its own.
    #[test]
    fn no_build_output_is_copied_at_any_depth() {
        let dir = tmp("skips_target");
        let (engine, project) = (dir.join("engine_src"), dir.join("proj"));
        fake_engine(&engine);
        fs::create_dir_all(&project).unwrap();

        let dest = vendor_engine(&project, &engine).expect("vendors");

        assert!(!dest.join("target").exists(), "top-level target/ copied");
        assert!(
            !dest.join("crates/kooch_core/target").exists(),
            "a nested target/ was copied — the recursion only checks the top",
        );
    }

    /// The engine's `assets/` is 13 MB and 12.7 MB of it is demo models.
    /// A project gets what it runs on, not the samples.
    #[test]
    fn assets_are_the_ones_a_game_runs_on_not_the_demos() {
        let dir = tmp("assets");
        let (engine, project) = (dir.join("engine_src"), dir.join("proj"));
        fake_engine(&engine);
        fs::create_dir_all(&project).unwrap();

        let dest = vendor_engine(&project, &engine).expect("vendors");

        assert!(dest.join("assets/materials/default.material").is_file());
        assert!(dest.join("assets/meshes/primitives/cube.glb").is_file());
        assert!(
            !dest.join("assets/meshes/demo.glb").exists(),
            "a demo model was vendored into every project",
        );
    }

    /// The distinction the whole feature turns on. An installed editor
    /// that ships source next to itself must still vendor, or nobody
    /// ever gets a self-contained project.
    #[test]
    fn shipping_source_next_to_the_editor_is_not_developing_the_engine() {
        let dir = tmp("dev_detection");
        let installed = dir.join("opt_kooch");
        fake_engine(&installed);
        assert!(
            is_engine_source(&installed),
            "the fixture should look like engine source — that is the point",
        );
        assert!(
            !running_from_engine_build(&installed),
            "the test binary does not live under this fixture's target/, \
             so this must read as an installed editor",
        );
    }

    /// The case a gitignored engine creates on every fresh clone of a
    /// game repo. If this does not work, "clone and open" is broken for
    /// everyone but the author.
    #[test]
    fn a_missing_engine_is_materialised_on_open() {
        let dir = tmp("materialise");
        let (engine, project) = (dir.join("editor_src"), dir.join("proj"));
        fake_engine(&engine);
        fs::create_dir_all(&project).unwrap();

        let state = ensure_current(&project, "0.0.0", Some(&engine)).unwrap();

        assert_eq!(state, VendorState::Materialised);
        assert!(
            project
                .join("engine/crates/kooch_core/src/lib.rs")
                .is_file()
        );
    }

    /// The case an editor update creates. The manifest's version is the
    /// only record of what the copy on disk is, so the comparison is
    /// against that and not against anything inside the copy.
    #[test]
    fn an_outdated_engine_is_replaced_wholesale() {
        let dir = tmp("replace");
        let (engine, project) = (dir.join("editor_src"), dir.join("proj"));
        fake_engine(&engine);
        fs::create_dir_all(project.join("engine/crates/gone/src")).unwrap();
        fs::write(project.join("engine/Cargo.toml"), "[workspace]").unwrap();
        fs::create_dir_all(project.join("engine/src")).unwrap();
        fs::write(project.join("engine/src/lib.rs"), "// old").unwrap();
        fs::write(project.join("engine/crates/gone/src/lib.rs"), "// removed").unwrap();

        let state = ensure_current(&project, "0.0.1", Some(&engine)).unwrap();

        assert_eq!(
            state,
            VendorState::Replaced {
                was: "0.0.1".to_owned()
            }
        );
        // 🔴 Replaced, not merged. A crate deleted from the engine that
        // survived here would still be compiled by the workspace glob,
        // and the error would name a crate nobody has touched in months.
        assert!(
            !project.join("engine/crates/gone").exists(),
            "a crate removed from the engine survived the update",
        );
    }

    /// Re-copying 8 MB on every open would make opening a project feel
    /// broken, and would stomp an engine someone is deliberately
    /// hacking on inside their project.
    #[test]
    fn a_current_engine_is_left_alone() {
        let dir = tmp("uptodate");
        let (engine, project) = (dir.join("editor_src"), dir.join("proj"));
        fake_engine(&engine);
        fs::create_dir_all(&project).unwrap();
        ensure_current(&project, "0.0.0", Some(&engine)).unwrap();

        let marker = project.join("engine/src/local_edit.rs");
        fs::write(&marker, "// mine").unwrap();

        let state = ensure_current(&project, editor_engine_version(), Some(&engine)).unwrap();

        assert_eq!(state, VendorState::UpToDate);
        assert!(marker.is_file(), "an up-to-date engine was re-copied");
    }

    /// An editor with no source to vendor from must not delete what the
    /// project already has. Opening a project should never leave it less
    /// buildable than it was.
    #[test]
    fn without_source_the_projects_own_copy_survives() {
        let dir = tmp("nosource");
        let (engine, project) = (dir.join("editor_src"), dir.join("proj"));
        fake_engine(&engine);
        fs::create_dir_all(&project).unwrap();
        ensure_current(&project, "0.0.0", Some(&engine)).unwrap();

        let state = ensure_current(&project, "0.0.1", None).unwrap();

        assert_eq!(state, VendorState::NoSourceAvailable);
        assert!(
            project
                .join("engine/crates/kooch_core/src/lib.rs")
                .is_file(),
            "opening with no engine source deleted the project's copy",
        );
    }

    /// Failing here is cheap; failing at `cargo build` in a project
    /// whose engine directory is half a copy is not, because nothing in
    /// that error mentions vendoring.
    #[test]
    fn a_directory_that_is_not_the_engine_is_refused_before_writing() {
        let dir = tmp("refuses");
        let (not_engine, project) = (dir.join("somewhere"), dir.join("proj"));
        fs::create_dir_all(&not_engine).unwrap();
        fs::create_dir_all(&project).unwrap();

        assert!(matches!(
            vendor_engine(&project, &not_engine),
            Err(VendorError::NotAnEngineRoot(_)),
        ));
        assert!(!project.join(VENDOR_DIR).exists(), "wrote before checking");
    }
}
