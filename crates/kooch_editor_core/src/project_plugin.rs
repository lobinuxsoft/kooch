//! Loading a project's own code into the editor.

use std::path::{Path, PathBuf};

use kooch_core::dynamic::{EngineHost, PluginLoader};
use kooch_core::resource::Resources;

mod reload;

pub use reload::{Changed, Reloaded};

/// Keeps loaded project plugins alive for as long as the project is open.
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

/// Loads the project's library, if it built one, and lets it declare its component types.
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

    // SAFETY: the library was produced by building the project the user asked to open, from its own
    // source.
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
        // Not fatal either way. The project still opens; it just shows
        // none of its own components until a build produces a library
        // this engine will load.
        Err(kooch_core::dynamic::PluginLoadError::Incompatible {
            reason: kooch_core::dynamic::Incompatibility::EngineVersion { .. },
            ..
        }) => {
            // 🔴 Expected, and self-correcting.
            tracing::info!(
                target: "kooch_editor_core::project_plugin",
                "the project's library was built against another engine version; \
                 rebuilding it is part of opening the project",
            );
        }
        Err(e) => tracing::warn!("{e}"),
    }

    resources.insert(plugins);
    registered_type_count(resources).saturating_sub(before)
}

/// Unloads every project plugin, dropping the types they declared.
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

/// Swaps the project's library for the one on disk right now.
pub fn reload_project_plugins(
    resources: &mut Resources,
    project_root: &Path,
    crate_name: &str,
) -> Reloaded {
    let before = resources
        .get::<kooch_ecs::component::DynamicTypeRegistry>()
        .cloned()
        .unwrap_or_default();

    unload_project_plugins(resources);
    let registered = load_project_plugin(resources, project_root, crate_name);

    if registered == 0 && !before.is_empty() {
        tracing::warn!(
            "the rebuilt library declared nothing — keeping the types from before the \
             reload, because a project with no components is what a failed build looks \
             like from here",
        );
        resources.insert(before);
        return Reloaded::default();
    }

    let after = resources
        .get::<kooch_ecs::component::DynamicTypeRegistry>()
        .cloned()
        .unwrap_or_default();
    let report = Reloaded::between(&before, &after);
    report.report();
    report
}

/// The source name a library's types were registered under.
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
mod tests;

#[cfg(test)]
mod staleness_tests;
