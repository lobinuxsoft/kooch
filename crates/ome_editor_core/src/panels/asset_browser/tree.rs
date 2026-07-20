//! Folder-tree model + rendering for the Asset Browser.
//!
//! Mirrors the on-disk project tree (Unity / Godot style): each source
//! root ("Project" = the crate root, "Engine" = shipped assets) is a
//! top-level node, folders nest below it, and every file is a leaf.
//! Files with a registered `.meta` are typed assets (single-click →
//! Inspector); the rest are plain files. Double-clicking any file opens
//! it in an external IDE with the project as the workspace, so Rust
//! source and `Cargo.toml` are editable. Writable (project) roots also
//! expose a right-click menu and inline rename.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use egui::collapsing_header::CollapsingState;
use ome_core::Guid;

use crate::actions::{EditorAction, NewFileKind};
use crate::drag_drop::DraggedAsset;
use crate::icons;
use crate::panels::inspector::AssetCatalogEntry;

/// A folder node in the tree. Rebuilt fresh each frame (project trees,
/// minus `target/`, are small enough that the fs walk is trivial).
pub(super) struct FolderNode {
    name: String,
    path: PathBuf,
    folders: BTreeMap<String, FolderNode>,
    files: Vec<FileLeaf>,
}

struct FileLeaf {
    name: String,
    path: PathBuf,
    /// `Some((guid, type_name))` when the file is a registered typed
    /// asset; `None` for plain files (source, config, …).
    asset: Option<(Guid, String)>,
}

/// In-progress inline rename: which path is being edited + the buffer.
#[derive(Clone)]
pub(super) struct RenameState {
    pub path: PathBuf,
    pub buffer: String,
    /// Whether the text field has grabbed focus yet (first frame only).
    pub focused: bool,
}

/// In-progress inline creation: the parent folder + what to create. The
/// name is typed before the file is generated, so scripts get the right
/// identifier substituted into their template.
#[derive(Clone)]
pub(super) struct PendingCreate {
    pub parent: PathBuf,
    pub kind: CreateKind,
    pub buffer: String,
    pub focused: bool,
}

/// What an inline creation produces.
#[derive(Clone, Copy)]
pub(super) enum CreateKind {
    Folder,
    Material,
    File(NewFileKind),
}

impl FolderNode {
    fn new(name: String, path: PathBuf) -> Self {
        Self {
            name,
            path,
            folders: BTreeMap::new(),
            files: Vec::new(),
        }
    }

    /// Walks `root_path` on disk, overlaying `entries` (the typed asset
    /// catalog) so registered files carry their GUID + type.
    fn build(root_path: &Path, entries: &[&AssetCatalogEntry]) -> Self {
        let by_path: HashMap<&Path, (Guid, &str)> = entries
            .iter()
            .map(|e| (e.path.as_path(), (e.guid, e.type_name.as_str())))
            .collect();
        let mut root = FolderNode::new(String::new(), root_path.to_path_buf());
        walk(&mut root, root_path, &by_path);
        root
    }

    /// `true` when this folder — or any descendant — has a file matching
    /// the (already lowercased) search needle. Empty is always a match.
    fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        self.files
            .iter()
            .any(|f| f.name.to_lowercase().contains(needle))
            || self.folders.values().any(|f| f.matches(needle))
    }
}

/// Recursively populates `node` from `dir`. Skips build/vcs noise
/// (`target/`, dot-folders) and `.meta` sidecars (shown via their asset).
fn walk(node: &mut FolderNode, dir: &Path, by_path: &HashMap<&Path, (Guid, &str)>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if name == "target" || name.starts_with('.') {
                continue;
            }
            let child = node
                .folders
                .entry(name.clone())
                .or_insert_with(|| FolderNode::new(name, path.clone()));
            walk(child, &path, by_path);
        } else {
            if name.ends_with(".meta") {
                continue;
            }
            let asset = by_path
                .get(path.as_path())
                .map(|(g, t)| (*g, (*t).to_owned()));
            node.files.push(FileLeaf { name, path, asset });
        }
    }
    node.files.sort_by(|a, b| a.name.cmp(&b.name));
}

/// State threaded through the recursive render.
pub(super) struct RenderCtx<'a> {
    pub needle: &'a str,
    pub selected_asset: &'a mut Option<Guid>,
    pub current_folder: &'a mut Option<PathBuf>,
    pub actions: &'a mut Vec<EditorAction>,
    pub rename: &'a mut Option<RenameState>,
    pub pending: &'a mut Option<PendingCreate>,
    /// `true` for the project root (writable: menus + folder targeting),
    /// `false` for the read-only engine root.
    pub writable: bool,
}

/// Renders one source root as a top-level collapsible node. `root_path`
/// doubles as the IDE workspace for files under it.
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
            // A selectable label (not a plain label) senses secondary
            // clicks, so the root's context menu actually opens.
            let resp = ui.selectable_label(false, format!("{} {}", icons::FOLDER_OPEN, label));
            if ctx.writable {
                let actions = &mut *ctx.actions;
                let rename = &mut *ctx.rename;
                let pending = &mut *ctx.pending;
                resp.context_menu(|ui| folder_menu(ui, &root, true, actions, rename, pending));
            }
        })
        .body(|ui| render_children(ui, &root, ctx, root_path));
}

fn render_children(ui: &mut egui::Ui, node: &FolderNode, ctx: &mut RenderCtx<'_>, root: &Path) {
    if ctx.pending.as_ref().is_some_and(|p| p.parent == node.path) {
        create_edit(ui, ctx.pending, ctx.actions);
    }
    for sub in node.folders.values() {
        if sub.matches(ctx.needle) {
            render_folder(ui, sub, ctx, root);
        }
    }
    for leaf in &node.files {
        if ctx.needle.is_empty() || leaf.name.to_lowercase().contains(ctx.needle) {
            render_leaf(ui, leaf, ctx, root);
        }
    }
}

fn render_folder(ui: &mut egui::Ui, node: &FolderNode, ctx: &mut RenderCtx<'_>, root: &Path) {
    if ctx.rename.as_ref().is_some_and(|r| r.path == node.path) {
        rename_edit(ui, &node.path, true, ctx.actions, ctx.rename);
        return;
    }

    let id = ui.make_persistent_id(("asset_folder", &node.path));
    let is_current = ctx.writable && ctx.current_folder.as_deref() == Some(node.path.as_path());

    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
    if !ctx.needle.is_empty() || ctx.pending.as_ref().is_some_and(|p| p.parent == node.path) {
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
            let pending = &mut *ctx.pending;
            resp.context_menu(|ui| folder_menu(ui, node, writable, actions, rename, pending));
        })
        .body(|ui| render_children(ui, node, ctx, root));
}

fn render_leaf(ui: &mut egui::Ui, leaf: &FileLeaf, ctx: &mut RenderCtx<'_>, root: &Path) {
    if ctx.rename.as_ref().is_some_and(|r| r.path == leaf.path) {
        rename_edit(ui, &leaf.path, false, ctx.actions, ctx.rename);
        return;
    }

    let icon = match &leaf.asset {
        Some((_, type_name)) => type_icon(type_name),
        None => file_icon(&leaf.name),
    };
    let selected = leaf
        .asset
        .as_ref()
        .is_some_and(|(g, _)| *ctx.selected_asset == Some(*g));

    let mut resp = ui.selectable_label(selected, format!("{icon} {}", leaf.name));

    // Typed assets are drag sources for the Inspector's asset slots
    // (#439). The sense is upgraded in place rather than wrapping the row
    // in `dnd_drag_source`, so click / double-click / context menu keep
    // working on the same response.
    if let Some((guid, type_name)) = &leaf.asset {
        resp = resp.interact(egui::Sense::click_and_drag());
        resp.dnd_set_drag_payload(DraggedAsset {
            guid: *guid,
            type_name: type_name.clone(),
        });
        if resp.dragged() {
            draw_drag_preview(ui, icon, &leaf.name);
        }
    }
    let resp = resp.on_hover_text(leaf.path.display().to_string());

    if resp.clicked() {
        // Single-click selects a typed asset for the Inspector; plain
        // files have nothing to inspect.
        if let Some((guid, _)) = &leaf.asset {
            *ctx.selected_asset = if selected { None } else { Some(*guid) };
        }
    }
    if resp.double_clicked() {
        ctx.actions.push(EditorAction::OpenInIde {
            root: root.to_path_buf(),
            file: leaf.path.clone(),
        });
    }

    let writable = ctx.writable;
    let actions = &mut *ctx.actions;
    let rename = &mut *ctx.rename;
    resp.context_menu(|ui| leaf_menu(ui, leaf, writable, root, actions, rename));
}

fn folder_menu(
    ui: &mut egui::Ui,
    node: &FolderNode,
    writable: bool,
    actions: &mut Vec<EditorAction>,
    rename: &mut Option<RenameState>,
    pending: &mut Option<PendingCreate>,
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
    // "New X" starts an inline name prompt rather than creating straight
    // away, so the typed name lands in the file name and the template.
    let mut start = |kind: CreateKind| {
        *pending = Some(PendingCreate {
            parent: node.path.clone(),
            kind,
            buffer: String::new(),
            focused: false,
        });
    };
    if ui
        .button(format!("{} New Folder", icons::FOLDER_PLUS))
        .clicked()
    {
        start(CreateKind::Folder);
        ui.close();
    }
    if ui
        .button(format!("{} New Material", icons::FADERS))
        .clicked()
    {
        start(CreateKind::Material);
        ui.close();
    }
    ui.menu_button(format!("{} New Script / Scene", icons::PLUS), |ui| {
        for (label, kind) in [
            (
                "Component (Rust)",
                CreateKind::File(NewFileKind::RustComponent),
            ),
            ("System (Rust)", CreateKind::File(NewFileKind::RustSystem)),
            ("Scene", CreateKind::File(NewFileKind::Scene)),
        ] {
            if ui.button(label).clicked() {
                start(kind);
                ui.close();
            }
        }
    });
    // The synthetic root node has an empty name; it is not itself
    // renamable / deletable (that would target the crate root).
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
    leaf: &FileLeaf,
    writable: bool,
    root: &Path,
    actions: &mut Vec<EditorAction>,
    rename: &mut Option<RenameState>,
) {
    if ui.button("Open in IDE").clicked() {
        actions.push(EditorAction::OpenInIde {
            root: root.to_path_buf(),
            file: leaf.path.clone(),
        });
        ui.close();
    }
    if writable {
        ui.separator();
        if ui.button("Rename").clicked() {
            // Edit the name before the first extension; the suffix
            // (`.ron`, `.rs`, `.ome_material.ron`, …) is re-attached on
            // commit.
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
        // Rust sources can be (re)registered as components / systems by
        // rescanning src/ — the editor detects which they are.
        if leaf.name.ends_with(".rs")
            && ui
                .button(format!("{} Register scripts", icons::PLUS))
                .clicked()
        {
            actions.push(EditorAction::RegisterScripts);
            ui.close();
        }
    }
    if let Some((guid, _)) = &leaf.asset {
        ui.separator();
        if ui.button("Copy GUID").clicked() {
            ui.ctx().copy_text(guid.to_string());
            ui.close();
        }
    }
    ui.separator();
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

/// Builds the new file/folder name from the edited stem, re-attaching a
/// file's extension suffix. Returns `None` for an empty edit.
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

/// Renders the inline creation name field. Commits on Enter (non-empty),
/// cancels on Escape / focus loss.
fn create_edit(
    ui: &mut egui::Ui,
    pending: &mut Option<PendingCreate>,
    actions: &mut Vec<EditorAction>,
) {
    let Some(p) = pending.as_mut() else {
        return;
    };
    let resp = ui.add(
        egui::TextEdit::singleline(&mut p.buffer)
            .desired_width(f32::INFINITY)
            .hint_text(create_hint(p.kind)),
    );
    if !p.focused {
        resp.request_focus();
        p.focused = true;
    }
    let name = p.buffer.trim().to_owned();
    let kind = p.kind;
    let parent = p.parent.clone();

    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if !escape && !resp.lost_focus() {
        return;
    }
    if resp.lost_focus() && enter && !escape && !name.is_empty() {
        // A new component / system auto-registers so it works without a
        // manual step; the CreateFile above runs first (writes the file),
        // then RegisterScripts rescans src/ and picks it up.
        let auto_register = matches!(
            kind,
            CreateKind::File(NewFileKind::RustComponent | NewFileKind::RustSystem)
        );
        actions.push(match kind {
            CreateKind::Folder => EditorAction::CreateFolder { parent, name },
            CreateKind::Material => EditorAction::CreateMaterial {
                folder: parent,
                name,
            },
            CreateKind::File(kind) => EditorAction::CreateFile {
                folder: parent,
                name,
                kind,
            },
        });
        if auto_register {
            actions.push(EditorAction::RegisterScripts);
        }
    }
    *pending = None;
}

fn create_hint(kind: CreateKind) -> &'static str {
    match kind {
        CreateKind::Folder => "Folder name…",
        CreateKind::Material => "Material name…",
        CreateKind::File(NewFileKind::RustComponent) => "Component name (e.g. Health)…",
        CreateKind::File(NewFileKind::RustSystem) => "System name (e.g. Movement)…",
        CreateKind::File(NewFileKind::Scene) => "Scene name…",
    }
}

/// Floating chip that follows the cursor while an asset row is dragged.
///
/// Painted on the tooltip layer so it clears every panel, including the
/// Inspector it is being dragged towards.
fn draw_drag_preview(ui: &egui::Ui, icon: &str, name: &str) {
    let Some(pos) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);

    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("asset_drag_preview"));
    let painter = ui.ctx().layer_painter(layer);
    let color = ui.visuals().strong_text_color();
    let galley = painter.layout_no_wrap(
        format!("{icon} {name}"),
        egui::FontId::proportional(13.0),
        color,
    );
    let text_pos = pos + egui::vec2(14.0, 8.0);
    let bg = egui::Rect::from_min_size(text_pos, galley.size()).expand(4.0);
    painter.rect_filled(bg, 3.0, ui.visuals().panel_fill);
    painter.galley(text_pos, galley, color);
}

/// Icon for a typed asset by its canonical type name.
fn type_icon(type_name: &str) -> &'static str {
    match type_name {
        "ome_render::meshlet::asset::MeshletMesh" => icons::CUBE,
        "ome_render::material::asset::Material" => icons::FADERS,
        _ => icons::STACK,
    }
}

/// Icon for a plain (non-asset) file by extension.
fn file_icon(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "rs" => icons::TERMINAL,
        "toml" | "lock" => icons::GEAR,
        "ome_scene" => icons::TREE_STRUCTURE,
        _ => icons::LIST_BULLETS,
    }
}
