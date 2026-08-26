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
    /// What DLSS put beside the executable, when the preset asked for
    /// it (#536): the runtime blob and NVIDIA's notices.
    ///
    /// Reported rather than silent — a file that appears in a build
    /// folder without being mentioned is a file its author deletes, and
    /// the notices are the one that must not be deleted.
    pub dlss: Vec<PathBuf>,
    /// The mingw C++ runtime a cross-compiled Windows build carries
    /// (#962), empty for every other build.
    ///
    /// Reported for the same reason `dlss` is: three DLLs appear in the
    /// folder, and an unexplained file beside a game is a file somebody
    /// deletes — these are the ones it cannot start without.
    pub runtime: Vec<PathBuf>,
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
    /// A runtime file the build cannot start without could not be found
    /// or copied.
    ///
    /// Separate from `Io` because the fix is not a filesystem one: it
    /// names a missing toolchain piece and what to install.
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
///
/// `known` is every extension some registered loader claims — the
/// allowlist, derived rather than maintained.
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
    // Both paths canonical-ish before comparing: `output_dir: "."`
    // joins to `<root>/.`, which is the project root and is not equal to
    // it as written.
    //
    // 🔴 Each platform gets its own subfolder. Sharing one would have the
    // second build overwrite the first's pack and manifest while leaving
    // both executables behind — a folder that looks like it holds two
    // games and holds one and a half.
    //
    // 🔴 The *base* is checked, not just the platform folder. Appending
    // `linux/` to a dangerous `output_dir` would make it look safe —
    // `output_dir: "src"` becomes `src/linux`, which is not `src` and
    // would sail past a guard that only saw the final path, while
    // packaging still emptied a folder inside the project's source.
    let root = normalise(project_root);
    let base = normalise(&project_root.join(&preset.output_dir));
    guard(&base, &root)?;
    let dir = base.join(platform.folder());
    prepare(&dir, &root)?;

    let dest_binary = dir.join(preset.binary_name(crate_name, platform));
    std::fs::copy(binary, &dest_binary)?;
    keep_executable(binary, &dest_binary);

    // The manifest travels, so the game can open the scene the project
    // says it opens with (#808).
    //
    // 🔴 Beside the executable and NOT in the pack. The scene bootstrap
    // reads it before the asset system exists — a game that failed to
    // open its pack still has to find its scene — and it holds no
    // authoring state worth protecting: a name, a version, and the
    // window size the same game shows in its title bar.
    //
    // Missing is not an error. A project built before this had no
    // manifest beside its binary, and the convention below is what such
    // a build has always used.
    let manifest = project_root.join(kooch_core::scene_paths::PROJECT_MANIFEST_FILE);
    if manifest.is_file() {
        std::fs::copy(
            &manifest,
            dir.join(kooch_core::scene_paths::PROJECT_MANIFEST_FILE),
        )?;
    }

    // 🔴 Scenes go in the pack too. A scene is the structure of the
    // whole game — every entity, every component, every value, including
    // the names of components its author wrote — and leaving it in plain
    // RON beside an encrypted pack protects the textures and publishes
    // the design.
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

    // 🔴 mingw's C++ runtime, for a Windows build cross-compiled from
    // Linux (#962). Without it the folder looks complete and the game
    // stops at a Windows dialog naming a DLL — on someone else's
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

/// Refuses a path that is the project, or inside something the project
/// owns.
///
/// Applied to the output base *and* to the platform folder under it:
/// either one landing in the project's own tree is a folder packaging
/// would empty.
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
///
/// Engine first, then the project — so a project file of the same name
/// replaces the engine's rather than colliding, and the replacement is
/// reported.
///
/// # 🔴 The engine's assets are filtered by what the game references
///
/// The first version copied a fixed list — `materials` and
/// `meshes/primitives` — borrowed from the *vendoring* allowlist. That
/// list answers "what source does a project need to build", which is a
/// different question from "what does this game draw", and it guessed
/// wrong: a scene using the engine's `suzanne.glb` shipped without it and
/// rendered nothing, with no error, because a missing GUID is silent.
///
/// So the engine's tree is walked whole and then cut down to the GUIDs
/// the project's own scenes and prefabs actually name. That is smaller
/// than the curated list would ever be — the engine's 13 MB of assets are
/// mostly demos — and it cannot be wrong about a mesh somebody used.
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

/// Every guid the game can reach, followed to a fixed point.
///
/// 🔴 The graph has DEPTH, and this used to read one level of it. A
/// scene names a material and the material names a texture; collecting
/// only what scenes and prefabs say ships the material and leaves the
/// texture behind. A missing guid is silent, so the game starts and
/// samples the 1x1 white fallback — a textured surface that renders like
/// somebody authored it flat. Reported from a build made for the
/// handheld, and the earlier shape of this same bug is recorded two
/// functions down.
///
/// # How it walks
///
/// An index of `guid -> path` built once over both trees, then a
/// worklist from the roots. Each guid is visited **once**: a texture
/// forty materials share is queued once, resolved once, and its file
/// read once. That is what makes a cycle — two prefabs naming each
/// other — terminate rather than hang the build.
///
/// The roots are every project asset that travels, not just its
/// documents. The project ships whole, so any file in it can reach into
/// the engine's tree, which is exactly what a project material with an
/// engine texture does.
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

    // 🔴 The roots are the project's FILES, not their guids. The whole
    // project ships, so anything in it can reach into the engine's tree
    // — and a scene has no sidecar of its own, so keying the roots off
    // guids drops the very documents the walk exists to start from.
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
///
/// A path is looked for in the project first and in the engine second,
/// which is the order everything else here resolves names in: the
/// project is the author and wins.
///
/// ⚠️ A declared path that resolves to nothing is REPORTED, not fatal.
/// It is the same class of mistake this whole walk exists to prevent —
/// an asset that does not ship — so it must not be silent; but refusing
/// to build over one stale line in a manifest is a worse trade than a
/// build that says what it could not find.
/// ⚠️ Guids, not files. Reading the declared file directly looks like
/// the thorough thing to do and is unreachable: a declared file in the
/// PROJECT is already read as a root, and one in the engine has a
/// sidecar, so its guid goes on the queue and the walk opens it there.
/// Written, found untestable, removed.
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
///
/// 🔴 Verified across every loader this engine registers: nothing
/// binary embeds a guid. A `.glb` is geometry and its material is
/// assigned by the scene, not by the file.
///
/// This exists for cost, not for correctness — reading a 16 MB texture
/// to search it for a 36-character string is waste repeated once per
/// asset. ⚠️ **A binary format that starts referencing assets has to
/// come off this list**, and `binary_formats_reference_nothing` in the
/// tests is what fails when one is added without doing so.
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
///
/// 🔴 `Guid::to_string()` writes no hyphens and a scene file writes them,
/// so comparing the two as they come found nothing — every engine asset
/// looked unreferenced and none of them shipped. The first symptom was a
/// game whose Suzanne was missing, which is also what the curated list
/// this replaced used to do.
fn normalise_guid(guid: &str) -> String {
    guid.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Every `"xxxxxxxx-xxxx-…"` in `text`.
///
/// A shape match rather than a parse: the document format is RON and the
/// guids sit inside `AssetRef(guid: Some("…"))`, but reaching for them
/// through the type would mean deserialising a scene here — and a scene
/// that fails to deserialise must not stop a build from packaging the
/// rest.
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
///
/// 🔴 An allowlist, and derived: a file no registered loader claims is
/// not an asset. A `.blend` exported beside its `.glb` — which is what
/// everyone does — is source, not content, and shipping it hands 80 MB
/// and the editable original to whoever opens the pack.
///
/// Derived rather than a list somebody maintains, because a list is a
/// second place to add an asset type and the day it is forgotten the
/// type stops shipping with no error. `known_extensions()` comes from
/// the loaders themselves.
///
/// `.meta` always travels: it is not an asset, it is how one is found.
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

#[cfg(test)]
mod package_tests;
