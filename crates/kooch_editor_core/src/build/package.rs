//! Laying out a shipped game (#758).
//!
//! ```text
//! dist/
//!   mygame.x86_64       the executable, named for its target
//!   assets.kpack        the scenes and everything they reference
//! ```
//!
//! Two files, because the scenes are in the pack as well — see below.
//! With `pack_assets` off they land beside the executable instead, at the
//! paths they have in the project (`assets/scenes/default.scene`), which
//! is the layout the runtime reads either way.
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
use super::platform::Platform;

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
    /// What DLSS put beside the executable, when the preset asked for it (#536): the runtime blob
    /// and NVIDIA's notices.
    pub dlss: Vec<PathBuf>,
    /// The mingw C++ runtime a cross-compiled Windows build carries (#962), empty for every other
    /// build.
    pub runtime: Vec<PathBuf>,
    /// Project assets that shadowed an engine asset of the same name.
    pub shadowed: Vec<String>,
}

#[derive(Debug)]
pub enum PackageError {
    Io(std::io::Error),
    /// A runtime file the build cannot start without could not be found or copied.
    Runtime(String),
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
            Self::Runtime(why) => write!(f, "{why}"),
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
    platform: Platform,
    known: &[String],
    project_root: &Path,
    engine_root: Option<&Path>,
    binary: &Path,
    crate_name: &str,
    key: &PackKey,
) -> Result<Package, PackageError> {
    if !binary.is_file() {
        return Err(PackageError::NoBinary(binary.to_path_buf()));
    }
    // Both paths canonical-ish before comparing: `output_dir: "."` joins to `<root>/.`, which is
    // the project root and is not equal to it as written.
    let root = normalise(project_root);
    let base = normalise(&project_root.join(&preset.output_dir));
    guard(&base, &root)?;
    let dir = base.join(platform.folder());
    prepare(&dir, &root)?;

    let dest_binary = dir.join(preset.binary_name(crate_name, platform));
    std::fs::copy(binary, &dest_binary)?;
    keep_executable(binary, &dest_binary);

    // The manifest travels, so the game can open the scene the project says it opens with (#808).
    let manifest = project_root.join(kooch_core::scene_paths::PROJECT_MANIFEST_FILE);
    if manifest.is_file() {
        std::fs::copy(
            &manifest,
            dir.join(kooch_core::scene_paths::PROJECT_MANIFEST_FILE),
        )?;
    }

    // 🔴 Scenes go in the pack too. A scene is the structure of the whole game — every entity, every
    // component, every value, including the names of components its author wrote — and leaving it
    // in plain RON beside an encrypted pack protects the textures and publishes the design.
    let (files, shadowed) = collect_assets(project_root, engine_root, known);
    let scene_count = files
        .iter()
        .filter(|(name, _)| {
            std::path::Path::new(name)
                .extension()
                .is_some_and(|e| e == kooch_core::scene_paths::SCENE_EXTENSION)
        })
        .count();

    let pack = match preset.pack_assets {
        true => Some(write_pack(&dir.join(PACK_FILE), &files, key)?),
        false => {
            // Loose, for working out why a build behaves differently from
            // the editor: the files are right there to look at.
            for (name, source) in &files {
                let to = dir.join(name);
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(source, &to)?;
            }
            None
        }
    };

    // #536 — NVIDIA's runtime blob and its notices, for a build that
    // asked for DLSS. Nothing for every other build.
    let dlss = super::dlss::ship(preset, platform, &dir)?;

    // 🔴 mingw's C++ runtime, for a Windows build cross-compiled from Linux (#962). Without it the
    // folder looks complete and the game stops at a Windows dialog naming a DLL — on someone else's
    // machine, which is the whole point of making a build.
    let runtime = super::mingw::ship(platform, &dir).map_err(PackageError::Runtime)?;

    Ok(Package {
        dir,
        binary: dest_binary,
        pack,
        assets: files.len() - scene_count,
        scenes: scene_count,
        dlss,
        runtime,
        shadowed,
    })
}

/// `a/b/./c` → `a/b/c`, and `a/b/../c` → `a/c`.
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
fn prepare(dir: &Path, project_root: &Path) -> Result<(), PackageError> {
    guard(dir, project_root)?;

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

/// Refuses a path that is the project, or inside something the project owns.
fn guard(dir: &Path, project_root: &Path) -> Result<(), PackageError> {
    let unsafe_place = dir == project_root
        || PROJECT_OWNED
            .iter()
            .any(|owned| dir.starts_with(project_root.join(owned)));
    match unsafe_place {
        true => Err(PackageError::UnsafeOutput(dir.to_path_buf())),
        false => Ok(()),
    }
}

/// Every asset that travels, as `(name in the pack, file on disk)`.
fn collect_assets(
    project_root: &Path,
    engine_root: Option<&Path>,
    known: &[String],
) -> (Vec<(String, PathBuf)>, Vec<String>) {
    // The engine's tree, cut down to what this project's documents name.
    let mut files = Vec::new();
    if let Some(engine) = engine_root {
        walk(&engine.join("assets"), "assets", &mut files);
        files.retain(|(name, _)| travels(name, known));
        let wanted = reachable_guids(project_root, engine_root, known);
        let by_name: std::collections::HashMap<&str, PathBuf> = files
            .iter()
            .map(|(name, path)| (name.as_str(), path.clone()))
            .collect();
        // A sidecar is judged by the asset it describes, not by its own
        // absence of a guid.
        let keep: Vec<bool> = files
            .iter()
            .map(|(name, path)| {
                let asset = name.strip_suffix(".meta").unwrap_or(name);
                let described = by_name.get(asset).unwrap_or(path);
                guid_of(described).is_some_and(|guid| wanted.contains(&guid))
            })
            .collect();
        let mut keep = keep.into_iter();
        files.retain(|_| keep.next().unwrap_or(false));
    }
    let engine_names: std::collections::HashSet<String> =
        files.iter().map(|(name, _)| name.clone()).collect();

    let mut project = Vec::new();
    walk(&project_root.join("assets"), "assets", &mut project);
    project.retain(|(name, _)| travels(name, known));

    // The project is the author and wins. Refusing the build instead
    // would mean a name nobody chose — the engine's — could stop a game
    // from being made.
    let mut shadowed = Vec::new();
    for (name, path) in project {
        if engine_names.contains(&name) {
            // Reported once per asset, not once per file: a `.meta`
            // shadowing its own asset's `.meta` is the same event said
            // twice.
            if !name.ends_with(".meta") {
                shadowed.push(name.clone());
            }
            if let Some(slot) = files.iter_mut().find(|(n, _)| *n == name) {
                slot.1 = path;
                continue;
            }
        }
        files.push((name, path));
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));
    (files, shadowed)
}

/// Files that live under `assets/` and are not the game's.
const AUTHORING_ONLY: [&str; 1] = [super::preset::BUILD_PRESET_EXTENSION];

/// Every guid the game can reach, followed to a fixed point.
fn reachable_guids(
    project_root: &Path,
    engine_root: Option<&Path>,
    known: &[String],
) -> std::collections::HashSet<String> {
    // guid -> file, over both trees: a reference crosses from the
    // project into the engine, and inside the engine from a material to
    // its texture.
    let mut index: std::collections::HashMap<String, PathBuf> = std::collections::HashMap::new();
    let mut catalogue = |root: &Path| {
        let mut files = Vec::new();
        walk(root, "assets", &mut files);
        for (name, path) in files {
            // A sidecar has no identity of its own; it carries its
            // asset's.
            if name.ends_with(".meta") || !travels(&name, known) {
                continue;
            }
            if let Some(guid) = guid_of(&path) {
                index.insert(guid, path);
            }
        }
    };
    if let Some(engine) = engine_root {
        catalogue(&engine.join("assets"));
    }
    // Second, so a project asset shadowing an engine one by name owns
    // the index entry as well as the pack.
    catalogue(&project_root.join("assets"));

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut read: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut queue: Vec<String> = Vec::new();

    // Assets the manifest declares because only code names them. Each is
    // a root of this walk like a document is, so declaring a material
    // brings the textures it points at.
    queue.extend(declared_roots(project_root, engine_root));

    // 🔴 The roots are the project's FILES, not their guids. The whole project ships, so anything in
    // it can reach into the engine's tree — and a scene has no sidecar of its own, so keying the
    // roots off guids drops the very documents the walk exists to start from.
    let mut project_files = Vec::new();
    walk(&project_root.join("assets"), "assets", &mut project_files);
    for (name, path) in project_files {
        if !travels(&name, known) {
            continue;
        }
        if let Some(text) = read_if_text(&path) {
            read.insert(path);
            queue.extend(guids_in(&text));
        }
    }

    while let Some(guid) = queue.pop() {
        if !seen.insert(guid.clone()) {
            continue;
        }
        let Some(path) = index.get(&guid) else {
            // Named by something and present in no tree: a dangling
            // reference, which is the author's problem and not a reason
            // to stop packaging.
            continue;
        };
        // Once per file, whatever names it — a texture forty materials
        // share is opened once.
        if !read.insert(path.clone()) {
            continue;
        }
        if let Some(text) = read_if_text(path) {
            queue.extend(guids_in(&text));
        }
    }
    seen
}

/// The manifest's `build.include` list, resolved to files.
fn declared_roots(project_root: &Path, engine_root: Option<&Path>) -> Vec<String> {
    let Ok(manifest) = crate::project::ProjectManifest::load(project_root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for declared in &manifest.build.include {
        let relative = Path::new(declared.trim_start_matches('/'));
        let candidates = [
            Some(project_root.join(relative)),
            engine_root.map(|engine| engine.join(relative)),
        ];
        match candidates.into_iter().flatten().find(|path| path.is_file()) {
            Some(path) => found.extend(guid_of(&path)),
            None => tracing::warn!(
                target: "kooch_editor_core::build",
                declared = %declared,
                "the manifest declares an asset for the build and no such file exists in \
                 the project or the engine; it will be missing from the game",
            ),
        }
    }
    found
}

/// Extensions whose bytes cannot name another asset.
pub(super) const OPAQUE_FORMATS: [&str; 7] = ["png", "jpg", "jpeg", "glb", "gltf", "bin", "kpack"];

/// The file's text, or `None` when it cannot name anything.
fn read_if_text(path: &Path) -> Option<String> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if OPAQUE_FORMATS.contains(&extension.as_str()) {
        return None;
    }
    // Still fallible: an unlisted binary reads as invalid UTF-8 and is
    // skipped rather than mis-parsed.
    std::fs::read_to_string(path).ok()
}

/// A guid as bytes only, so two spellings of one id compare equal.
fn normalise_guid(guid: &str) -> String {
    guid.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Every `"xxxxxxxx-xxxx-…"` in `text`.
fn guids_in(text: &str) -> Vec<String> {
    text.split('"')
        .filter(|candidate| {
            candidate.len() == 36
                && candidate.split('-').map(str::len).eq([8, 4, 4, 4, 12])
                && candidate.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        })
        .map(normalise_guid)
        .collect()
}

/// The guid recorded beside `path`, if it has a sidecar.
fn guid_of(path: &Path) -> Option<String> {
    let meta = kooch_core::asset_meta::read_meta(path).ok()?;
    Some(normalise_guid(&meta.guid.to_string()))
}

/// Whether a file travels into the build.
fn travels(name: &str, known: &[String]) -> bool {
    let stem = name.strip_suffix(".meta").unwrap_or(name);
    if authoring_only(stem) {
        return false;
    }
    let lower = stem.to_ascii_lowercase();
    // What a loader claims, plus what the runtime reads by path — a
    // scene has no loader and a game without one starts empty.
    known
        .iter()
        .map(String::as_str)
        .chain(kooch_core::scene_paths::READ_BY_PATH)
        .any(|ext| lower.ends_with(&format!(".{}", ext.to_ascii_lowercase())))
}

/// Whether `name` is authoring configuration rather than game content.
fn authoring_only(name: &str) -> bool {
    let name = name.strip_suffix(".meta").unwrap_or(name);
    AUTHORING_ONLY
        .iter()
        .any(|ext| name.ends_with(&format!(".{ext}")))
}

/// Collects every file under `dir` as `prefix`-relative names.
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

#[cfg(test)]
mod package_tests;
