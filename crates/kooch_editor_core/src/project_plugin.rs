//! Loading a project's own code into the editor.
//!
//! The standalone editor cannot compile a project's component types, so
//! until now the only way to see them was to launch the project as a
//! separate process and mirror it over HTTP. This is the other route:
//! the project builds a `dylib`, and the editor loads it directly.
//!
//! # What a plugin may do *in the editor*
//!
//! Declare types — nothing else. The editor never runs gameplay: its
//! schedule belongs to the editor's own frame, and a project's systems
//! would be running against a world the user is editing. So the host
//! handed to the plugin here has no schedule, and a plugin that tries to
//! register a system is refused with a log line rather than quietly
//! having its systems dropped.
//!
//! Running them is what Play is for, in the project's own process.
//!
//! # Absent is normal
//!
//! A project with no library — every project created before this
//! existed — loads exactly as it did before. Nothing here fails an open;
//! the worst case is a debug line saying there was nothing to load.

use std::path::{Path, PathBuf};

use kooch_core::dynamic::{EngineHost, PluginLoader};
use kooch_core::resource::Resources;

/// Keeps loaded project plugins alive for as long as the project is open.
///
/// Dropping this unloads the libraries, which invalidates every vtable
/// pointing into them — so it lives as a resource and is replaced only
/// when a project closes or reloads.
#[derive(Default)]
pub struct ProjectPlugins {
    loader: Option<PluginLoader>,
    /// Path of each library currently loaded, for diagnostics.
    loaded: Vec<PathBuf>,
}

impl ProjectPlugins {
    /// Paths of the libraries currently loaded.
    pub fn loaded(&self) -> &[PathBuf] {
        &self.loaded
    }

    /// Whether anything is loaded.
    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
    }
}

/// Where a project's built library would be, if it has one.
///
/// Debug before release: the editor is a development tool, and a stale
/// release artefact next to a fresh debug one would be the wrong answer.
/// Both are checked so a project built either way is found.
pub fn library_path(project_root: &Path, crate_name: &str) -> Option<PathBuf> {
    let file = library_file_name(crate_name);
    ["debug", "release"]
        .iter()
        .map(|profile| project_root.join("target").join(profile).join(&file))
        .find(|candidate| candidate.exists())
}

/// Platform file name for a Rust dynamic library.
fn library_file_name(crate_name: &str) -> String {
    // Cargo replaces dashes with underscores in artefact names.
    let stem = crate_name.replace('-', "_");
    #[cfg(target_os = "windows")]
    {
        format!("{stem}.dll")
    }
    #[cfg(target_os = "macos")]
    {
        format!("lib{stem}.dylib")
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        format!("lib{stem}.so")
    }
}

/// Loads the project's library, if it built one, and lets it declare its
/// component types.
///
/// Returns how many types the registry gained. Never fails an open: a
/// missing library is the normal case for a project that predates this,
/// and a broken one is reported without taking the editor down.
/// Says so when the library predates the sources it was built from.
///
/// The editor loads this `.so`; it does not build it. So a component
/// written and not yet compiled simply is not in the menu, and the only
/// symptom is a type that "does not exist" — time spent looking at the
/// derive, at `registrations.rs` and at the `#[reflect]` attribute, none
/// of which are wrong.
///
/// Compares against the newest `.rs` under `src/`. A false alarm costs a
/// line in the Console; staying quiet costs the search above.
fn stale_source(project_root: &Path, library: &Path) -> Option<std::path::PathBuf> {
    let built = library.metadata().and_then(|m| m.modified()).ok()?;
    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    let mut stack = vec![project_root.join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && let Ok(modified) = entry.metadata().and_then(|m| m.modified())
                && newest.as_ref().is_none_or(|(seen, _)| modified > *seen)
            {
                newest = Some((modified, path));
            }
        }
    }

    newest
        .filter(|(modified, _)| *modified > built)
        .map(|(_, path)| path)
}

/// Says so when the library predates its sources.
fn warn_if_stale(project_root: &Path, library: &Path) {
    if let Some(newer) = stale_source(project_root, library) {
        tracing::warn!(
            newer = %newer.display(),
            "the project library is older than its sources, so the editor is \
             showing the components of the last build — rebuild the project \
             and reopen it, or a component you just wrote will not appear"
        );
    }
}

pub fn load_project_plugin(
    resources: &mut Resources,
    project_root: &Path,
    crate_name: &str,
) -> usize {
    let Some(path) = library_path(project_root, crate_name) else {
        tracing::debug!(
            project = %project_root.display(),
            "no project library to load — the project defines no components for the editor"
        );
        return 0;
    };

    warn_if_stale(project_root, &path);

    let before = registered_type_count(resources);

    let mut plugins = resources.remove::<ProjectPlugins>().unwrap_or_default();
    let loader = plugins.loader.get_or_insert_with(PluginLoader::new);

    // SAFETY: the library was produced by building the project the user
    // asked to open, from its own source. Loading it runs its
    // initialisers, which is the same trust the user extends by pressing
    // Play — and the loader refuses anything not built against this
    // engine by this compiler before calling into it.
    let plugin = unsafe { loader.load(&path) };

    match plugin {
        Ok(mut plugin) => {
            tracing::info!(plugin = plugin.name(), path = %path.display(), "loaded project plugin");
            // No schedule: in the editor a plugin declares types, it does
            // not run. `add_system` logs a refusal rather than silently
            // dropping the system.
            let mut host = EngineHost::running(resources);
            plugin.build(&mut host);
            plugins.loaded.push(path);
        }
        Err(e) => {
            // Not fatal. The project still opens; it just shows none of
            // its own components, and the reason says why.
            tracing::warn!("{e}");
        }
    }

    resources.insert(plugins);
    registered_type_count(resources).saturating_sub(before)
}

/// Unloads every project plugin, dropping the types they declared.
///
/// Called when a project closes. Instances already placed on entities
/// are left alone — they live in `DynamicComponents` keyed by name, and
/// a reload is about to re-register the very same types.
pub fn unload_project_plugins(resources: &mut Resources) {
    if let Some(mut types) = resources.remove::<kooch_ecs::component::DynamicTypeRegistry>() {
        if let Some(plugins) = resources.get::<ProjectPlugins>() {
            for path in &plugins.loaded {
                if let Some(source) = source_of(path) {
                    types.remove_source(&source);
                }
            }
        }
        resources.insert(types);
    }
    resources.remove::<ProjectPlugins>();
}

/// The source name a library's types were registered under.
///
/// Matches what the ECS bridge derives — the first path segment of the
/// type name, which for `my_game::Health` is `my_game`, the crate.
fn source_of(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    Some(stem.strip_prefix("lib").unwrap_or(stem).to_owned())
}

/// How many dynamic types are registered right now.
fn registered_type_count(resources: &Resources) -> usize {
    resources
        .get::<kooch_ecs::component::DynamicTypeRegistry>()
        .map_or(0, |r| r.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_names_follow_cargo() {
        let name = library_file_name("my-game");
        assert!(
            name.contains("my_game"),
            "cargo replaces dashes with underscores, got {name}"
        );
        #[cfg(target_os = "linux")]
        assert_eq!(name, "libmy_game.so");
    }

    #[test]
    fn a_project_without_a_library_yields_none() {
        let dir = std::env::temp_dir().join("kooch_no_lib_test");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(library_path(&dir, "absent"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Opening a project that has no library must not fail, and must not
    /// register anything.
    #[test]
    fn loading_nothing_is_not_an_error() {
        let dir = std::env::temp_dir().join("kooch_no_lib_load_test");
        std::fs::create_dir_all(&dir).unwrap();

        let mut resources = Resources::new();
        assert_eq!(load_project_plugin(&mut resources, &dir, "absent"), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_source_name_matches_what_the_bridge_derives() {
        assert_eq!(
            source_of(Path::new("/p/target/debug/libmy_game.so")).as_deref(),
            Some("my_game")
        );
        assert_eq!(
            source_of(Path::new("/p/target/debug/my_game.dll")).as_deref(),
            Some("my_game")
        );
    }
}

#[cfg(test)]
mod staleness_tests {
    use super::stale_source;

    /// 🔴 A component written and not yet compiled is simply absent from
    /// the add-component menu, and nothing says why: the editor loads
    /// this library, it does not build it.
    ///
    /// Reported from a real session — the `.so` was 21 minutes older than
    /// the component that "did not exist", and the time went into the
    /// derive, `registrations.rs` and the `#[reflect]` attribute, none of
    /// which were wrong.
    #[test]
    fn a_source_newer_than_the_library_is_reported() {
        let dir = std::env::temp_dir().join("kooch_stale_plugin_test");
        let src = dir.join("src").join("components");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&src).expect("temp dirs");

        let library = dir.join("libproject.so");
        std::fs::write(&library, b"not really a library").expect("write library");

        // Written after, which is exactly the reported situation.
        let component = src.join("input.rs");
        std::fs::write(&component, b"pub struct PlayerInput;").expect("write source");
        filetime_bump(&component);

        assert_eq!(
            stale_source(&dir, &library).as_deref(),
            Some(component.as_path()),
            "a source newer than the library went unreported"
        );

        // And the other way round: a fresh build is quiet, or the warning
        // fires on every open and stops meaning anything.
        let rebuilt = dir.join("librebuilt.so");
        std::fs::write(&rebuilt, b"newer").expect("write");
        filetime_bump(&rebuilt);
        assert_eq!(
            stale_source(&dir, &rebuilt),
            None,
            "a library newer than every source was reported as stale"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No sources at all is not a complaint: a project that defines no
    /// components is a normal project.
    #[test]
    fn a_project_with_no_sources_is_quiet() {
        let dir = std::env::temp_dir().join("kooch_stale_plugin_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let library = dir.join("libproject.so");
        std::fs::write(&library, b"x").expect("write");

        assert_eq!(stale_source(&dir, &library), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Rewrites `path` until its mtime is strictly later than it was, so
    /// the test does not depend on filesystem timestamp granularity —
    /// which on some mounts is a whole second.
    fn filetime_bump(path: &std::path::Path) {
        let before = path.metadata().and_then(|m| m.modified()).expect("mtime");
        for _ in 0..100 {
            std::fs::write(path, b"touched").expect("rewrite");
            let now = path.metadata().and_then(|m| m.modified()).expect("mtime");
            if now > before {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("could not make {} newer", path.display());
    }
}
