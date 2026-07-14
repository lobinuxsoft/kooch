//! Folder-tree model + rendering for the Asset Browser.
//!
//! Mirrors the on-disk `assets/` hierarchy (Unity / Godot style): each
//! source root ("Project", "Engine") is a top-level node, folders nest
//! below it, and assets are selectable leaves. Selecting a leaf drives
//! the Inspector; clicking a folder (in a writable root) sets it as the
//! drag-and-drop import target.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::collapsing_header::CollapsingState;
use ome_core::Guid;

use crate::icons;
use crate::panels::inspector::AssetCatalogEntry;

/// A folder node in the asset tree. Built fresh each frame from the
/// catalog (asset counts are small, so the rebuild is trivial).
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

    /// Builds a tree of `entries` relative to `root_path`. Path segments
    /// before the filename become nested folders; the filename becomes a
    /// leaf.
    fn build(root_path: &Path, entries: &[&AssetCatalogEntry]) -> Self {
        let mut root = FolderNode::new(String::new(), root_path.to_path_buf());
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
            });
        }
        root
    }

    /// `true` when this folder — or any descendant — holds an asset whose
    /// name matches the (already lowercased) search needle.
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

/// State threaded through the recursive render.
pub(super) struct RenderCtx<'a> {
    pub needle: &'a str,
    pub selected_asset: &'a mut Option<Guid>,
    pub current_folder: &'a mut Option<PathBuf>,
    /// `true` for project roots (folders selectable as import targets),
    /// `false` for the read-only engine root.
    pub writable: bool,
}

/// Renders one source root (e.g. "Project") as a top-level collapsible
/// node containing its folder tree.
pub(super) fn render_root(
    ui: &mut egui::Ui,
    label: &str,
    root_path: &Path,
    entries: &[&AssetCatalogEntry],
    ctx: &mut RenderCtx<'_>,
) {
    let root = FolderNode::build(root_path, entries);
    let id = ui.make_persistent_id(("asset_root", root_path));
    CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            ui.label(format!("{} {}", icons::FOLDER_OPEN, label));
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
    let id = ui.make_persistent_id(("asset_folder", &node.path));
    let is_current = ctx.writable && ctx.current_folder.as_deref() == Some(node.path.as_path());

    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
    // A search auto-expands folders so matches inside them stay visible.
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
        })
        .body(|ui| render_children(ui, node, ctx));
}

fn render_leaf(ui: &mut egui::Ui, leaf: &AssetLeaf, ctx: &mut RenderCtx<'_>) {
    let selected = *ctx.selected_asset == Some(leaf.guid);
    let label = format!("{} {}", type_icon(&leaf.type_name), leaf.name);
    let resp = ui
        .selectable_label(selected, label)
        .on_hover_text(&leaf.type_name);
    if resp.clicked() {
        *ctx.selected_asset = if selected { None } else { Some(leaf.guid) };
    }
}

/// Icon per asset type. Generic `STACK` for anything not explicitly mapped.
fn type_icon(type_name: &str) -> &'static str {
    match type_name {
        "ome_render::meshlet::asset::MeshletMesh" => icons::CUBE,
        "ome_render::material::asset::Material" => icons::FADERS,
        _ => icons::STACK,
    }
}
