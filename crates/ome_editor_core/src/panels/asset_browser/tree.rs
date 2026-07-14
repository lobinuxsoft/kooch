//! Folder-tree model + rendering for the Asset Browser.
//!
//! Mirrors the on-disk `assets/` hierarchy (Unity / Godot style): each
//! source root ("Project", "Engine") is a top-level node, folders nest
//! below it, and assets are selectable leaves. Writable (project) roots
//! also walk the filesystem so empty folders appear, and expose a
//! right-click menu (new folder / material, rename, duplicate, delete,
//! reveal) plus inline rename.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::collapsing_header::CollapsingState;
use ome_core::Guid;

use crate::actions::EditorAction;
use crate::icons;
use crate::panels::inspector::AssetCatalogEntry;

/// A folder node in the asset tree. Built fresh each frame (asset counts
/// and project trees are small, so the rebuild + fs walk is trivial).
pub(super) struct FolderNode {
    name: String,
    path: PathBuf,
    folders: BTreeMap<String, FolderNode>,
    assets: Vec<AssetLeaf>,
}

struct AssetLeaf {
    guid: Guid,
    name: String,
    type_name: String,
    path: PathBuf,
}

/// In-progress inline rename: which path is being edited + the buffer.
#[derive(Clone)]
pub(super) struct RenameState {
    pub path: PathBuf,
    pub buffer: String,
    /// Whether the text field has grabbed focus yet (first frame only).
    pub focused: bool,
}

impl FolderNode {
    fn new(name: String, path: PathBuf) -> Self {
        Self {
            name,
            path,
            folders: BTreeMap::new(),
            assets: Vec::new(),
        }
    }

    /// Builds a tree of `entries` relative to `root_path`. When
    /// `scan_dirs` is set the filesystem is walked first so empty folders
    /// appear (needed for the writable project root).
    fn build(root_path: &Path, entries: &[&AssetCatalogEntry], scan_dirs: bool) -> Self {
        let mut root = FolderNode::new(String::new(), root_path.to_path_buf());
        if scan_dirs {
            walk_dirs(&mut root, root_path);
        }
        for entry in entries {
            let rel = entry.path.strip_prefix(root_path).unwrap_or(&entry.path);
            let comps: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            if comps.is_empty() {
                continue;
            }
            let folder_count = comps.len() - 1;
            let mut node = &mut root;
            for seg in &comps[..folder_count] {
                let child_path = node.path.join(seg);
                node = node
                    .folders
                    .entry(seg.clone())
                    .or_insert_with(|| FolderNode::new(seg.clone(), child_path));
            }
            node.assets.push(AssetLeaf {
                guid: entry.guid,
                name: entry.display_name.clone(),
                type_name: entry.type_name.clone(),
                path: entry.path.clone(),
            });
        }
        root
    }

    /// `true` when this folder — or any descendant — matches the (already
    /// lowercased) search needle. Empty is always a match.
    fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        self.assets
            .iter()
            .any(|a| a.name.to_lowercase().contains(needle))
            || self.folders.values().any(|f| f.matches(needle))
    }
}

/// Recursively adds every real subdirectory of `dir` to `node` so empty
/// folders show. Hidden folders (dot-prefixed) are skipped.
fn walk_dirs(node: &mut FolderNode, dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let child = node
            .folders
            .entry(name.clone())
            .or_insert_with(|| FolderNode::new(name, path.clone()));
        walk_dirs(child, &path);
    }
}

/// State threaded through the recursive render.
pub(super) struct RenderCtx<'a> {
    pub needle: &'a str,
    pub selected_asset: &'a mut Option<Guid>,
    pub current_folder: &'a mut Option<PathBuf>,
    pub actions: &'a mut Vec<EditorAction>,
    pub rename: &'a mut Option<RenameState>,
    /// `true` for project roots (writable: menus + folder targeting),
    /// `false` for the read-only engine root.
    pub writable: bool,
}

/// Renders one source root as a top-level collapsible node.
pub(super) fn render_root(
    ui: &mut egui::Ui,
    label: &str,
    root_path: &Path,
    entries: &[&AssetCatalogEntry],
    ctx: &mut RenderCtx<'_>,
) {
    let root = FolderNode::build(root_path, entries, ctx.writable);
    let id = ui.make_persistent_id(("asset_root", root_path));
    CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            let resp = ui.label(format!("{} {}", icons::FOLDER_OPEN, label));
            if ctx.writable {
                let actions = &mut *ctx.actions;
                let rename = &mut *ctx.rename;
                resp.context_menu(|ui| folder_menu(ui, &root, true, actions, rename));
            }
        })
        .body(|ui| render_children(ui, &root, ctx));
}

fn render_children(ui: &mut egui::Ui, node: &FolderNode, ctx: &mut RenderCtx<'_>) {
    for sub in node.folders.values() {
        if sub.matches(ctx.needle) {
            render_folder(ui, sub, ctx);
        }
    }
    for leaf in &node.assets {
        if ctx.needle.is_empty() || leaf.name.to_lowercase().contains(ctx.needle) {
            render_leaf(ui, leaf, ctx);
        }
    }
}

fn render_folder(ui: &mut egui::Ui, node: &FolderNode, ctx: &mut RenderCtx<'_>) {
    if ctx.rename.as_ref().is_some_and(|r| r.path == node.path) {
        rename_edit(ui, &node.path, true, ctx.actions, ctx.rename);
        return;
    }

    let id = ui.make_persistent_id(("asset_folder", &node.path));
    let is_current = ctx.writable && ctx.current_folder.as_deref() == Some(node.path.as_path());

    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
    if !ctx.needle.is_empty() {
        state.set_open(true);
    }
    state
        .show_header(ui, |ui| {
            let resp =
                ui.selectable_label(is_current, format!("{} {}", icons::FOLDER_OPEN, node.name));
            if resp.clicked() && ctx.writable {
                *ctx.current_folder = Some(node.path.clone());
            }
            let writable = ctx.writable;
            let actions = &mut *ctx.actions;
            let rename = &mut *ctx.rename;
            resp.context_menu(|ui| folder_menu(ui, node, writable, actions, rename));
        })
        .body(|ui| render_children(ui, node, ctx));
}

fn render_leaf(ui: &mut egui::Ui, leaf: &AssetLeaf, ctx: &mut RenderCtx<'_>) {
    if ctx.rename.as_ref().is_some_and(|r| r.path == leaf.path) {
        rename_edit(ui, &leaf.path, false, ctx.actions, ctx.rename);
        return;
    }

    let selected = *ctx.selected_asset == Some(leaf.guid);
    let resp = ui
        .selectable_label(
            selected,
            format!("{} {}", type_icon(&leaf.type_name), leaf.name),
        )
        .on_hover_text(&leaf.type_name);
    if resp.clicked() {
        *ctx.selected_asset = if selected { None } else { Some(leaf.guid) };
    }
    let writable = ctx.writable;
    let actions = &mut *ctx.actions;
    let rename = &mut *ctx.rename;
    resp.context_menu(|ui| leaf_menu(ui, leaf, writable, actions, rename));
}

fn folder_menu(
    ui: &mut egui::Ui,
    node: &FolderNode,
    writable: bool,
    actions: &mut Vec<EditorAction>,
    rename: &mut Option<RenameState>,
) {
    if !writable {
        if ui.button("Reveal in file manager").clicked() {
            actions.push(EditorAction::RevealInFileManager {
                path: node.path.clone(),
            });
            ui.close();
        }
        return;
    }
    if ui
        .button(format!("{} New Folder", icons::FOLDER_PLUS))
        .clicked()
    {
        actions.push(EditorAction::CreateFolder {
            parent: node.path.clone(),
            name: "New Folder".to_owned(),
        });
        ui.close();
    }
    if ui
        .button(format!("{} New Material", icons::FADERS))
        .clicked()
    {
        actions.push(EditorAction::CreateMaterial {
            folder: node.path.clone(),
            name: "New Material".to_owned(),
        });
        ui.close();
    }
    // The synthetic root node has an empty name; it is not itself
    // renamable / deletable (that would target the assets/ root).
    if !node.name.is_empty() {
        ui.separator();
        if ui.button("Rename").clicked() {
            *rename = Some(RenameState {
                path: node.path.clone(),
                buffer: node.name.clone(),
                focused: false,
            });
            ui.close();
        }
        if ui.button(format!("{} Delete", icons::TRASH)).clicked() {
            actions.push(EditorAction::DeleteFolder {
                path: node.path.clone(),
            });
            ui.close();
        }
    }
    ui.separator();
    if ui.button("Reveal in file manager").clicked() {
        actions.push(EditorAction::RevealInFileManager {
            path: node.path.clone(),
        });
        ui.close();
    }
}

fn leaf_menu(
    ui: &mut egui::Ui,
    leaf: &AssetLeaf,
    writable: bool,
    actions: &mut Vec<EditorAction>,
    rename: &mut Option<RenameState>,
) {
    if writable {
        if ui.button("Rename").clicked() {
            // Edit the name before the first extension; the suffix
            // (`.ron`, `.ome_material.ron`, …) is re-attached on commit.
            let stem = leaf.name.split('.').next().unwrap_or(&leaf.name).to_owned();
            *rename = Some(RenameState {
                path: leaf.path.clone(),
                buffer: stem,
                focused: false,
            });
            ui.close();
        }
        if ui.button(format!("{} Duplicate", icons::COPY)).clicked() {
            actions.push(EditorAction::DuplicateAsset {
                path: leaf.path.clone(),
            });
            ui.close();
        }
        if ui.button(format!("{} Delete", icons::TRASH)).clicked() {
            actions.push(EditorAction::DeleteAsset {
                path: leaf.path.clone(),
            });
            ui.close();
        }
        ui.separator();
    }
    if ui.button("Copy GUID").clicked() {
        ui.ctx().copy_text(leaf.guid.to_string());
        ui.close();
    }
    if ui.button("Reveal in file manager").clicked() {
        actions.push(EditorAction::RevealInFileManager {
            path: leaf.path.clone(),
        });
        ui.close();
    }
}

/// Renders the inline rename text field. Commits on Enter, cancels on
/// Escape or focus loss.
fn rename_edit(
    ui: &mut egui::Ui,
    path: &Path,
    is_folder: bool,
    actions: &mut Vec<EditorAction>,
    rename: &mut Option<RenameState>,
) {
    let Some(state) = rename.as_mut() else {
        return;
    };
    let resp = ui.add(egui::TextEdit::singleline(&mut state.buffer).desired_width(f32::INFINITY));
    if !state.focused {
        resp.request_focus();
        state.focused = true;
    }
    let buffer = state.buffer.clone();

    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if !escape && !resp.lost_focus() {
        return;
    }
    if resp.lost_focus() && enter && !escape {
        if let Some(new_name) = commit_name(path, &buffer, is_folder) {
            actions.push(if is_folder {
                EditorAction::RenameFolder {
                    path: path.to_path_buf(),
                    new_name,
                }
            } else {
                EditorAction::RenameAsset {
                    path: path.to_path_buf(),
                    new_name,
                }
            });
        }
    }
    *rename = None;
}

/// Builds the new file/folder name from the edited stem, re-attaching an
/// asset's extension suffix. Returns `None` for an empty edit.
fn commit_name(path: &Path, buffer: &str, is_folder: bool) -> Option<String> {
    let trimmed = buffer.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_folder {
        return Some(trimmed.to_owned());
    }
    let filename = path.file_name()?.to_str()?;
    let suffix = filename.find('.').map(|i| &filename[i..]).unwrap_or("");
    Some(format!("{trimmed}{suffix}"))
}

/// Icon per asset type. Generic `STACK` for anything not explicitly mapped.
fn type_icon(type_name: &str) -> &'static str {
    match type_name {
        "ome_render::meshlet::asset::MeshletMesh" => icons::CUBE,
        "ome_render::material::asset::Material" => icons::FADERS,
        _ => icons::STACK,
    }
}
