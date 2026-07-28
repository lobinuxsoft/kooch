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

use ome_core::dynamic::{EngineHost, PluginLoader};
use ome_core::resource::Resources;

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
    if let Some(mut types) = resources.remove::<ome_ecs::component::DynamicTypeRegistry>() {
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
        .get::<ome_ecs::component::DynamicTypeRegistry>()
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
        let dir = std::env::temp_dir().join("ome_no_lib_test");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(library_path(&dir, "absent"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Opening a project that has no library must not fail, and must not
    /// register anything.
    #[test]
    fn loading_nothing_is_not_an_error() {
        let dir = std::env::temp_dir().join("ome_no_lib_load_test");
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
