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

use ome_core::resource::Resources;

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
    s.push_str("// AUTO-GENERATED by the Oh My Engine editor — do not edit by hand.\n");
    s.push_str("// Regenerated when you create or register components / systems.\n");
    s.push_str("#![allow(unused_imports, unused_variables, dead_code)]\n\n");
    s.push_str("use oh_my_engine::ome_ecs::component::ComponentRegistry;\n");
    s.push_str("use oh_my_engine::prelude::*;\n\n");
    for f in files {
        s.push_str(&format!("#[path = \"{}\"]\nmod {};\n", f.rel, f.module));
    }
    s.push('\n');
    s.push_str("/// Editor-managed plugin: registers project components + systems.\n");
    s.push_str("///\n");
    s.push_str("/// `run_systems` gates gameplay systems: `true` in the game build,\n");
    s.push_str("/// `false` in the editor build (so nothing runs while you edit).\n");
    s.push_str("/// Components always register, so they appear in the Inspector.\n");
    s.push_str("pub struct ProjectRegistrations {\n");
    s.push_str("    pub run_systems: bool,\n");
    s.push_str("}\n\n");
    s.push_str("impl Plugin for ProjectRegistrations {\n");
    s.push_str("    fn build(&self, app: &mut App) {\n");
    s.push_str("        app.add_system(Stage::Startup, register_components);\n");
    s.push_str("        if self.run_systems {\n");
    for f in files {
        for sys in &f.systems {
            s.push_str(&format!(
                "            app.add_system(Stage::Update, {}::{});\n",
                f.module, sys
            ));
        }
    }
    s.push_str("        }\n");
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
    s.push_str("}\n");
    s
}

/// Ensures `src/main.rs` includes + installs the `registrations` module.
/// Regenerates it from the scaffold when missing; otherwise injects the
/// two wiring lines (`mod registrations;` + the `add_plugin` call)
/// non-destructively if they are absent.
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
    let mut lines: Vec<String> = content.lines().map(str::to_owned).collect();
    let mut changed = false;

    if !content.contains("mod registrations;") {
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
mod tests {
    use super::{detect, module_name};

    #[test]
    fn detects_component_and_system() {
        let src = "\
#[derive(Default, Reflect)]
pub struct Health {}
impl Component for Health {}

pub fn movement(resources: &mut Resources) {}
";
        let (components, systems) = detect(src);
        assert_eq!(components, vec!["Health".to_owned()]);
        assert_eq!(systems, vec!["movement".to_owned()]);
    }

    #[test]
    fn module_name_flattens_nested_paths() {
        assert_eq!(module_name("player_health.rs"), "player_health");
        assert_eq!(module_name("enemies/ai.rs"), "enemies_ai");
    }
}
