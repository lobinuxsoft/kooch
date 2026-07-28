//! Drawing the tree: roots, folders and leaves.

use std::path::Path;

use egui::collapsing_header::CollapsingState;

use crate::actions::EditorAction;
use crate::drag_drop::DraggedAsset;
use crate::icons;

use super::model::{FileLeaf, FolderNode};
use super::menus::{folder_menu, leaf_menu};
use super::naming::{create_edit, rename_edit};
use super::visuals::{draw_drag_preview, file_icon, type_icon};
use super::RenderCtx;

pub(super) fn render_children(ui: &mut egui::Ui, node: &FolderNode, ctx: &mut RenderCtx<'_>, root: &Path) {
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

pub(super) fn render_folder(ui: &mut egui::Ui, node: &FolderNode, ctx: &mut RenderCtx<'_>, root: &Path) {
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

pub(super) fn render_leaf(ui: &mut egui::Ui, leaf: &FileLeaf, ctx: &mut RenderCtx<'_>, root: &Path) {
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
