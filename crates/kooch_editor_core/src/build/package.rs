//! Laying out a shipped game (#758).
//!
//! ```text
//! dist/
//!   mygame              the executable
//!   scenes/             default.scene, and the rest
//!   assets.kpack        everything the scenes reference, encrypted
//! ```
//!
//! Takes an executable that is already built. Invoking cargo is a
//! separate concern with its own failures — a missing toolchain, ten
//! minutes of compiling — and keeping it out means this half is testable
//! without compiling anything.
//!
//! # 🔴 Two asset trees become one
//!
//! In the editor a project has two asset roots: the engine's and its own.
//! A shipped game has **one** — without `KOOCH_PROJECT_ROOT` the runtime
//! reads a single root (`src/lib.rs`) — so packaging is not "copy
//! `assets/`", it is merging `<engine>/assets/{materials,meshes/primitives}`
//! into `<project>/assets/` under one set of names.
//!
//! Get that wrong and the game starts, loads its scene, and draws
//! nothing: every engine GUID fails to resolve and no error says why.
//!
//! # ⚠️ `.meta` sidecars are not optional
//!
//! A scene references its assets by GUID and the GUID lives in the
//! `.meta` beside the file. A packer that filtered by extension would
//! produce a game that loads its scene and renders nothing — so
//! everything travels.

use std::path::{Path, PathBuf};

use kooch_pack::{PackKey, PackWriter};

use super::BuildPreset;

/// What came out of a packaging run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// The folder everything landed in.
    pub dir: PathBuf,
    /// The executable, inside `dir`.
    pub binary: PathBuf,
    /// The pack, when the preset asked for one.
    pub pack: Option<PathBuf>,
    /// How many asset files travelled.
    pub assets: usize,
    /// How many scene files travelled.
    pub scenes: usize,
    /// Project assets that shadowed an engine asset of the same name.
    ///
    /// Not an error — the project is the author and wins — but worth
    /// reporting, because the engine's version is simply gone from the
    /// build and nothing else would say so.
    pub shadowed: Vec<String>,
}

#[derive(Debug)]
pub enum PackageError {
    Io(std::io::Error),
    /// The built executable was not where it was said to be.
    NoBinary(PathBuf),
    /// The output folder is somewhere a build must not write.
    UnsafeOutput(PathBuf),
    Pack(kooch_pack::PackError),
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::NoBinary(p) => write!(f, "no executable at {}", p.display()),
            Self::UnsafeOutput(p) => write!(
                f,
                "{} looks like source, not an output folder — packaging would \
                 delete it. Point the preset's output at a folder of its own.",
                p.display(),
            ),
            Self::Pack(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PackageError {}

impl From<std::io::Error> for PackageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Name the pack takes, beside the executable.
pub const PACK_FILE: &str = "assets.kpack";

/// Assembles `preset`'s output folder from an already-built `binary`.
pub fn assemble(
    preset: &BuildPreset,
    project_root: &Path,
    engine_root: Option<&Path>,
    binary: &Path,
    crate_name: &str,
    key: &PackKey,
) -> Result<Package, PackageError> {
    if !binary.is_file() {
        return Err(PackageError::NoBinary(binary.to_path_buf()));
    }
    // Both paths canonical-ish before comparing: `output_dir: "."`
    // joins to `<root>/.`, which is the project root and is not equal to
    // it as written.
    let dir = normalise(&project_root.join(&preset.output_dir));
    prepare(&dir, &normalise(project_root))?;

    let dest_binary = dir.join(preset.binary_name(crate_name));
    std::fs::copy(binary, &dest_binary)?;
    keep_executable(binary, &dest_binary);

    let scenes = copy_tree(&project_root.join("scenes"), &dir.join("scenes"))?;

    let (files, shadowed) = collect_assets(project_root, engine_root);
    let pack = match preset.pack_assets {
        true => Some(write_pack(&dir.join(PACK_FILE), &files, key)?),
        false => {
            // Loose, for working out why a build behaves differently from
            // the editor: the files are right there to look at.
            let dest = dir.join("assets");
            for (name, source) in &files {
                let to = dest.join(name);
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(source, &to)?;
            }
            None
        }
    };

    Ok(Package {
        dir,
        binary: dest_binary,
        pack,
        assets: files.len(),
        scenes,
        shadowed,
    })
}

/// `a/b/./c` → `a/b/c`, and `a/b/../c` → `a/c`.
///
/// Lexical, not `canonicalize`: the output folder usually does not exist
/// yet, and `canonicalize` fails on a path that does not.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Directories a project keeps its own work in. An output folder may be
/// none of them, and may not be inside one.
const PROJECT_OWNED: [&str; 5] = ["src", "assets", "scenes", ".git", ".kooch"];

/// Empties the output folder, refusing anywhere that is not one.
///
/// 🔴 This deletes recursively and the path comes from a text field in a
/// preset, so it is checked two ways — and the first version had only the
/// second, which let `output_dir: "assets"` through:
///
/// 1. **By where it is.** The project root itself, or anything at or
///    inside `src/`, `assets/`, `scenes/`, `.git/`, `.kooch/`. Asking
///    "does it *contain* a `src`?" does not catch `src` itself, and a
///    test caught that this was exactly what happened.
/// 2. **By what it holds.** Somewhere outside the project that looks like
///    a source tree — a sibling checkout, a home directory.
fn prepare(dir: &Path, project_root: &Path) -> Result<(), PackageError> {
    let unsafe_place = dir == project_root
        || PROJECT_OWNED
            .iter()
            .any(|owned| dir.starts_with(project_root.join(owned)));
    if unsafe_place {
        return Err(PackageError::UnsafeOutput(dir.to_path_buf()));
    }

    if dir.exists() {
        if ["Cargo.toml", "src", ".git"]
            .iter()
            .any(|entry| dir.join(entry).exists())
        {
            return Err(PackageError::UnsafeOutput(dir.to_path_buf()));
        }
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    Ok(())
}

/// Every asset that travels, as `(name in the pack, file on disk)`.
///
/// Engine first, then the project — so a project file of the same name
/// replaces the engine's rather than colliding, and the replacement is
/// reported.
fn collect_assets(
    project_root: &Path,
    engine_root: Option<&Path>,
) -> (Vec<(String, PathBuf)>, Vec<String>) {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut shadowed = Vec::new();

    if let Some(engine) = engine_root {
        for entry in crate::engine_vendor::COPY_ASSETS {
            let from = engine.join("assets").join(entry);
            walk(&from, entry, &mut files);
        }
    }
    let engine_count = files.len();

    let mut project = Vec::new();
    walk(&project_root.join("assets"), "", &mut project);

    // The project is the author and wins. Refusing the build instead
    // would mean a name nobody chose — the engine's — could stop a game
    // from being made.
    for (name, path) in project {
        if let Some(slot) = files[..engine_count].iter().position(|(n, _)| *n == name) {
            shadowed.push(name.clone());
            files[slot] = (name, path);
        } else {
            files.push((name, path));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    (files, shadowed)
}

/// Files that live under `assets/` and are not the game's.
///
/// 🔴 A `.buildpreset` describes how to *make* the game — output folder,
/// target triple, cargo features. The game itself never reads one, and
/// shipping it hands anyone who opens the pack a description of how it is
/// built.
///
/// `.rendersettings` is the opposite and stays: exposure, ambient and
/// shadow distance are what the project *looks* like, and the renderer
/// reads them at startup.
const AUTHORING_ONLY: [&str; 1] = [super::preset::BUILD_PRESET_EXTENSION];

/// Whether `name` is authoring configuration rather than game content.
///
/// Sidecars go with whatever they describe: a `.buildpreset.meta` left
/// behind would be an orphan the pack scan counts and nothing resolves.
fn authoring_only(name: &str) -> bool {
    let name = name.strip_suffix(".meta").unwrap_or(name);
    AUTHORING_ONLY
        .iter()
        .any(|ext| name.ends_with(&format!(".{ext}")))
}

/// Collects every file under `dir` as `prefix`-relative names.
///
/// ⚠️ Everything, `.meta` included: a scene references assets by GUID and
/// the GUID lives in the sidecar. Everything except authoring-only files
/// — see [`authoring_only`].
fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if authoring_only(&name) {
            continue;
        }
        let joined = match prefix.is_empty() {
            true => name,
            false => format!("{prefix}/{name}"),
        };
        if path.is_dir() {
            walk(&path, &joined, out);
        } else {
            out.push((joined, path));
        }
    }
}

/// Writes the pack, returning where it landed.
fn write_pack(
    path: &Path,
    files: &[(String, PathBuf)],
    key: &PackKey,
) -> Result<PathBuf, PackageError> {
    let mut writer =
        PackWriter::new(std::fs::File::create(path)?, key).map_err(PackageError::Pack)?;
    for (name, source) in files {
        writer.add_file(name, source).map_err(PackageError::Pack)?;
    }
    writer.finish().map_err(PackageError::Pack)?;
    Ok(path.to_path_buf())
}

/// Carries the executable bit across, which `fs::copy` does on unix and
/// which nothing needs on Windows.
#[cfg(unix)]
fn keep_executable(from: &Path, to: &Path) {
    use std::os::unix::fs::PermissionsExt;

    // `fs::copy` already copies the mode; this is the repair for a
    // destination that existed with a stricter one.
    if let Ok(meta) = std::fs::metadata(from) {
        let mode = meta.permissions().mode() | 0o111;
        let _ = std::fs::set_permissions(to, std::fs::Permissions::from_mode(mode));
    }
}

#[cfg(not(unix))]
fn keep_executable(_from: &Path, _to: &Path) {}

/// Copies a directory, returning how many files landed. Missing is zero,
/// not an error: a project without `scenes/` is unusual, not broken.
fn copy_tree(from: &Path, to: &Path) -> Result<usize, PackageError> {
    let mut files = Vec::new();
    walk(from, "", &mut files);
    for (name, source) in &files {
        let dest = to.join(name);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(source, &dest)?;
    }
    Ok(files.len())
}

#[cfg(test)]
mod package_tests;
