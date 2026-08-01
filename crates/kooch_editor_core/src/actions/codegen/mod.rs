//! Codegen for the project's Rust registration glue.
//!
//! Scans `src/` for components (`impl Component for X`) and systems
//! (`pub fn f(_: &mut Resources)`), then rewrites the editor-owned
//! `src/registrations.rs` that declares the modules and registers
//! everything. The user's `main.rs` includes it with two lines; the
//! editor only touches `main.rs` to regenerate it when it goes missing.
//!
//! Detection is heuristic (line-based, not a full parse) — enough for
//! the generated templates and typical hand-written code. The Rhai /
//! convention-based auto-registration counterpart is tracked in #76.

use std::path::Path;

use kooch_core::resource::Resources;

use crate::project::{INITIAL_REGISTRATIONS, generate_main_rs};
use crate::project_state::ProjectState;

/// One source file's discovered registrables.
struct SourceFile {
    /// Path relative to `src/`, e.g. `enemies/ai.rs`.
    rel: String,
    /// Unique module identifier for the `#[path]` mod declaration.
    module: String,
    components: Vec<String>,
    systems: Vec<String>,
}

/// Regenerates `src/registrations.rs` from the project's `src/` tree and
/// ensures `main.rs` exists.
pub(crate) fn register_scripts(resources: &mut Resources) {
    let Some(project_root) = resources
        .get::<ProjectState>()
        .and_then(|ps| ps.active_project.as_ref().map(|ap| ap.root_path.clone()))
    else {
        tracing::warn!("register scripts: no active project");
        return;
    };
    let src = project_root.join("src");
    if !src.is_dir() {
        tracing::warn!(src = %src.display(), "register scripts: no src/ directory");
        return;
    }

    let mut files: Vec<SourceFile> = Vec::new();
    scan(&src, &src, &mut files);
    files.sort_by(|a, b| a.rel.cmp(&b.rel));

    let content = render_registrations(&files);
    let out = src.join("registrations.rs");
    match std::fs::write(&out, content) {
        Ok(()) => tracing::info!(
            file = %out.display(),
            components = files.iter().map(|f| f.components.len()).sum::<usize>(),
            systems = files.iter().map(|f| f.systems.len()).sum::<usize>(),
            "registrations regenerated",
        ),
        Err(e) => {
            tracing::error!(file = %out.display(), error = %e, "failed to write registrations")
        }
    }

    ensure_main_wired(&project_root, resources);
    ensure_features(&project_root);

    // Bring a project made before the library split up to date. Runs
    // last: it rewrites `main.rs`'s module line, which `ensure_main_wired`
    // above may have just recreated from the scaffold.
    let crate_name = resources
        .get::<ProjectState>()
        .and_then(|ps| {
            ps.active_project
                .as_ref()
                .map(|ap| ap.manifest.name.clone())
        })
        .map(|name| crate::project::sanitize_crate_name(&name))
        .unwrap_or_default();
    if !crate_name.is_empty() {
        migrate_to_library(&project_root, &crate_name);
    }
}

/// Adds engine features a project predates.
///
/// A feature that is missing does not fail to build — it compiles a host
/// without the system, so the component is authorable, mirrors to the
/// editor, draws its gizmo and does nothing. That failure mode is
/// indistinguishable from "the physics is broken", which is how it was
/// reported the last two times.
///
/// Deliberately narrow, like [`migrate_remote_host`]: it edits only the
/// exact dependency line the editor generated, and leaves a hand-written
/// manifest alone.
fn ensure_features(project_root: &Path) {
    /// Features added after the first scaffolds shipped, each with the
    /// component it makes real.
    const ADDED: &[(&str, &str)] = &[
        ("gravity", "PointGravity / AreaGravity"),
        ("camera", "CameraRig"),
        // Without this the generated lib.rs does not compile: the plugin
        // API it calls is behind this feature. Every project that
        // predates the library split needs it.
        (
            "dynamic",
            "loading this project's components into the editor",
        ),
    ];

    let manifest = project_root.join("Cargo.toml");
    let Ok(content) = std::fs::read_to_string(&manifest) else {
        return;
    };
    let mut out = content.clone();
    for (feature, what) in ADDED {
        let quoted = format!("\"{feature}\"");
        // `features = [` is the anchor rather than the whole line, so the
        // order of the existing features does not matter.
        let Some(start) = out.find("kooch = {") else {
            return;
        };
        let Some(list) = out[start..].find("features = [").map(|i| start + i) else {
            return;
        };
        let Some(end) = out[list..].find(']').map(|i| list + i) else {
            return;
        };
        if out[list..end].contains(&quoted) {
            continue;
        }
        out.insert_str(end, &format!(", {quoted}"));
        tracing::info!(
            feature = feature,
            enables = what,
            "Cargo.toml: added an engine feature this project predates",
        );
    }
    if out != content
        && let Err(e) = std::fs::write(&manifest, out)
    {
        tracing::error!(file = %manifest.display(), error = %e, "failed to update Cargo.toml");
    }
}

/// Brings a project created before the library split up to date.
///
/// Projects scaffolded earlier are a plain binary: `src/main.rs` with
/// `mod registrations;` and a manifest with no `[lib]`. The editor can
/// only load a project's component types from a `dylib`, so without this
/// every existing project would silently show none of its own
/// components — the failure mode being "the menu looks the same as
/// always", which nobody would report as a bug.
///
/// Three narrow edits, each skipped when already applied:
///
/// 1. `[lib] crate-type = ["rlib", "dylib"]` plus an explicit `[[bin]]`,
///    because declaring a lib target makes cargo stop inferring the bin.
/// 2. `src/lib.rs`, if absent.
/// 3. `mod registrations;` in `main.rs` becomes `use <crate>::registrations;`,
///    since the module now belongs to the library.
///
/// Nothing here rewrites gameplay code, and a manifest that does not
/// look like the editor's own is left alone — same rule as
/// [`ensure_features`].
pub(crate) fn migrate_to_library(project_root: &Path, crate_name: &str) {
    let manifest_path = project_root.join("Cargo.toml");
    let Ok(manifest) = std::fs::read_to_string(&manifest_path) else {
        return;
    };
    // Somebody else's manifest: leave it be.
    if !manifest.contains("kooch = {") {
        return;
    }

    if !manifest.contains("[lib]") {
        let targets = format!(
            "\n[lib]\ncrate-type = [\"rlib\", \"dylib\"]\n\n[[bin]]\nname = \"{crate_name}\"\npath = \"src/main.rs\"\n"
        );
        // Before `[dependencies]` so the file still reads top-down.
        let updated = match manifest.find("[dependencies]") {
            Some(at) => {
                let mut out = manifest.clone();
                out.insert_str(at, targets.trim_start_matches('\n'));
                out
            }
            None => format!("{manifest}{targets}"),
        };
        if let Err(e) = std::fs::write(&manifest_path, updated) {
            tracing::error!(file = %manifest_path.display(), error = %e, "failed to add lib target");
            return;
        }
        tracing::info!("Cargo.toml: added the library target the editor loads");
    }

    let lib_path = project_root.join("src").join("lib.rs");
    if !lib_path.exists() {
        let contents = crate::project::generate_lib_rs(crate_name);
        if let Err(e) = std::fs::write(&lib_path, contents) {
            tracing::error!(file = %lib_path.display(), error = %e, "failed to write lib.rs");
            return;
        }
        tracing::info!("src/lib.rs: created, so the editor can load this project's components");
    }

    let main_path = project_root.join("src").join("main.rs");
    let Ok(main) = std::fs::read_to_string(&main_path) else {
        return;
    };
    if !main.contains("mod registrations;") {
        return;
    }

    let use_line = format!("use {crate_name}::registrations;");
    let updated = if main.contains("::registrations;") {
        // Both lines present: the module was re-added beside the `use`,
        // which declares the name twice. Drop the stray `mod`.
        main.lines()
            .filter(|line| line.trim() != "mod registrations;")
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    } else {
        main.replace("mod registrations;", &use_line)
    };

    if updated == main {
        return;
    }
    match std::fs::write(&main_path, updated) {
        Ok(()) => {
            tracing::info!("src/main.rs: registrations now come from the project library")
        }
        Err(e) => {
            tracing::error!(file = %main_path.display(), error = %e, "failed to rewire main.rs")
        }
    }
}

/// Recursively collects source files under `dir` that declare a component
/// or system. `root` is the `src/` directory (for relative paths).
fn scan(root: &Path, dir: &Path, out: &mut Vec<SourceFile>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            scan(root, &path, out);
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".rs") {
            continue;
        }
        // Skip the crate roots + the generated file itself.
        if matches!(name.as_str(), "main.rs" | "lib.rs" | "registrations.rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (components, systems) = detect(&content);
        if components.is_empty() && systems.is_empty() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let module = module_name(&rel);
        out.push(SourceFile {
            rel,
            module,
            components,
            systems,
        });
    }
}

/// Heuristic detection of components (`impl Component for X`) and systems
/// (`pub fn f(…: &mut Resources)`) by scanning trimmed lines.
fn detect(content: &str) -> (Vec<String>, Vec<String>) {
    let mut components = Vec::new();
    let mut systems = Vec::new();
    for line in content.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("impl Component for ") {
            let name = ident_prefix(rest);
            if !name.is_empty() {
                components.push(name);
            }
        } else if let Some(rest) = l.strip_prefix("pub fn ") {
            if l.contains("&mut Resources") {
                let name = ident_prefix(rest);
                if !name.is_empty() {
                    systems.push(name);
                }
            }
        }
    }
    (components, systems)
}

fn ident_prefix(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// Unique module id from a src-relative path: `enemies/ai.rs` → `enemies_ai`.
fn module_name(rel: &str) -> String {
    let stem = rel.strip_suffix(".rs").unwrap_or(rel);
    let mut out: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn render_registrations(files: &[SourceFile]) -> String {
    let mut s = String::new();
    s.push_str("// AUTO-GENERATED by the Kóoch editor — do not edit by hand.\n");
    s.push_str("// Regenerated when you create or register components / systems.\n");
    s.push_str("#![allow(unused_imports, unused_variables, dead_code)]\n\n");
    s.push_str("use kooch::kooch_ecs::component::ComponentRegistry;\n");
    s.push_str("use kooch::prelude::*;\n\n");
    for f in files {
        s.push_str(&format!("#[path = \"{}\"]\nmod {};\n", f.rel, f.module));
    }
    s.push('\n');
    s.push_str("/// Editor-managed plugin: registers project components + systems.\n");
    s.push_str("///\n");
    s.push_str("/// Gameplay systems are always registered, wrapped in\n");
    s.push_str("/// `run_if_playing` so the `Playing` resource decides per frame\n");
    s.push_str("/// whether they run. `run_systems` only sets its starting value:\n");
    s.push_str("/// `true` in the game build, `false` while editing — and the\n");
    s.push_str("/// editor's Play button flips it live, with no rebuild.\n");
    s.push_str("pub struct ProjectRegistrations {\n");
    s.push_str("    pub run_systems: bool,\n");
    s.push_str("}\n\n");
    s.push_str("impl Plugin for ProjectRegistrations {\n");
    s.push_str("    fn build(&self, app: &mut App) {\n");
    s.push_str("        app.insert_resource(Playing(self.run_systems));\n");
    s.push_str("        app.add_system(Stage::Startup, register_components);\n");
    for f in files {
        for sys in &f.systems {
            s.push_str(&format!(
                "        app.add_system(Stage::Update, run_if_playing({}::{}));\n",
                f.module, sys
            ));
        }
    }
    s.push_str("    }\n}\n\n");
    s.push_str("/// Registers project components for serialization + the Inspector.\n");
    s.push_str("fn register_components(resources: &mut Resources) {\n");
    s.push_str("    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {\n");
    s.push_str("        return;\n");
    s.push_str("    };\n");
    for f in files {
        for c in &f.components {
            s.push_str(&format!(
                "    registry.register_cpu_reflected::<{}::{}>();\n",
                f.module, c
            ));
        }
    }
    s.push_str("}\n\n");

    // The other direction: describing these same types outward, so the
    // standalone editor can list them without compiling them. The engine
    // reads each type's own reflection, so this only has to name them.
    s.push_str("/// Describes project components to an editor that loads this library.\n");
    s.push_str("///\n");
    s.push_str("/// Called from `lib.rs` when the editor loads the project's dylib.\n");
    s.push_str("pub fn declare_components(engine: &mut dyn kooch::kooch_plugin_api::Engine) {\n");
    s.push_str("    use kooch::kooch_ecs::component::plugin_bridge::declare_component;\n");
    for f in files {
        for c in &f.components {
            s.push_str(&format!(
                "    let _ = declare_component::<{}::{}>(engine);\n",
                f.module, c
            ));
        }
    }
    s.push_str("}\n");
    s
}

/// Ensures `src/main.rs` includes + installs the `registrations` module.
/// Regenerates it from the scaffold when missing; otherwise injects the
/// two wiring lines (`mod registrations;` + the `add_plugin` call)
/// non-destructively if they are absent.
/// Rewrites a scaffold's `--remote` arm to the headless plugin set.
///
/// Earlier scaffolds built the remote host from `DefaultPlugins`, which
/// opens a window — so the project drew the same scene the editor was
/// already drawing, in a second window competing for focus. The match is
/// deliberately narrow: it only fires on the exact three lines the
/// editor itself generated, so a hand-written `main.rs` is left alone.
fn migrate_remote_host(content: &str) -> String {
    const OLD: &str = "\
        app.add_plugins(DefaultPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: false });
        app.add_plugin(kooch::kooch_remote::RemotePlugin::new());";
    const NEW: &str = "\
        app.add_plugins(RemoteHostPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: false });
        app.add_plugin(kooch::kooch_remote::RemotePlugin::new());";
    if !content.contains(OLD) {
        return content.to_owned();
    }
    tracing::info!("main.rs: migrated --remote to the headless host plugins");
    content.replace(OLD, NEW)
}

fn ensure_main_wired(project_root: &Path, resources: &Resources) {
    let src = project_root.join("src");
    let main = src.join("main.rs");

    if !main.exists() {
        let name = resources
            .get::<ProjectState>()
            .and_then(|ps| {
                ps.active_project
                    .as_ref()
                    .map(|ap| ap.manifest.name.clone())
            })
            .unwrap_or_else(|| "game".to_owned());
        if let Err(e) = std::fs::write(&main, generate_main_rs(&name)) {
            tracing::error!(file = %main.display(), error = %e, "failed to regenerate main.rs");
            return;
        }
        let reg = src.join("registrations.rs");
        if !reg.exists() {
            let _ = std::fs::write(&reg, INITIAL_REGISTRATIONS);
        }
        tracing::info!(file = %main.display(), "main.rs regenerated");
        return;
    }

    let Ok(content) = std::fs::read_to_string(&main) else {
        return;
    };
    let content = migrate_remote_host(&content);
    let mut lines: Vec<String> = content.lines().map(str::to_owned).collect();
    let mut changed = content != std::fs::read_to_string(&main).unwrap_or_default();

    // `use <crate>::registrations;` means the module belongs to the
    // project's library now. Re-adding `mod registrations;` beside it
    // declares the name twice and the project stops compiling — which is
    // exactly what happened the first time these two ran together.
    if !content.contains("mod registrations;") && !content.contains("::registrations;") {
        lines.insert(0, "mod registrations;".to_owned());
        lines.insert(1, String::new());
        changed = true;
    }
    if !content.contains("registrations::ProjectRegistrations") {
        // Install after `DefaultPlugins` if present, else right after
        // `App::new()` — both are stable anchors in a scaffold main.
        let anchor = lines
            .iter()
            .position(|l| l.contains("add_plugins("))
            .or_else(|| lines.iter().position(|l| l.contains("App::new()")));
        match anchor {
            Some(i) => {
                lines.insert(
                    i + 1,
                    "    app.add_plugin(registrations::ProjectRegistrations);".to_owned(),
                );
                changed = true;
            }
            None => tracing::warn!(
                file = %main.display(),
                "could not wire registrations (no App::new()/add_plugins found); add \
                 `app.add_plugin(registrations::ProjectRegistrations);` manually",
            ),
        }
    }
    if changed {
        let out = format!("{}\n", lines.join("\n"));
        match std::fs::write(&main, out) {
            Ok(()) => tracing::info!(file = %main.display(), "main.rs wired to registrations"),
            Err(e) => tracing::error!(file = %main.display(), error = %e, "failed to wire main.rs"),
        }
    }
}

#[cfg(test)]
mod tests;
