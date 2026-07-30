//! Drawing the tree: roots, folders and leaves.

use std::path::Path;

use egui::collapsing_header::CollapsingState;

use crate::actions::EditorAction;
use crate::drag_drop::DraggedAsset;
use crate::icons;
use ome_ecs::entity::Entity;

use super::RenderCtx;
use super::menus::{folder_menu, leaf_menu};
use super::model::{FileLeaf, FolderNode};
use super::naming::{create_edit, rename_edit};
use super::nav::AssetRow;
use super::visuals::{draw_drag_preview, file_icon, type_icon};

pub(super) fn render_children(
    ui: &mut egui::Ui,
    node: &FolderNode,
    ctx: &mut RenderCtx<'_>,
    root: &Path,
) {
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

pub(super) fn render_folder(
    ui: &mut egui::Ui,
    node: &FolderNode,
    ctx: &mut RenderCtx<'_>,
    root: &Path,
) {
    if ctx.rename.as_ref().is_some_and(|r| r.path == node.path) {
        rename_edit(ui, &node.path, true, ctx.actions, ctx.rename);
        return;
    }

    let id = ui.make_persistent_id(("asset_folder", &node.path));
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
    if !ctx.needle.is_empty() || ctx.pending.as_ref().is_some_and(|p| p.parent == node.path) {
        state.set_open(true);
    }
    // The keyboard's request, applied here because this is the only place
    // the folder's persistent id exists.
    if let Some(open) = ctx.nav.take_toggle_for(&node.path) {
        state.set_open(open);
    }
    let is_cursor = ctx.nav.is_cursor(&node.path);
    // Read before `show_header` consumes `state`, and used for both the
    // row record and the glyph so the two cannot disagree.
    let is_open = state.is_open();
    ctx.nav.rows.push(AssetRow {
        path: node.path.clone(),
        is_folder: true,
        open: is_open,
    });
    state
        .show_header(ui, |ui| {
            // Open or closed, so the glyph says what the arrow beside it
            // says. Both were `FOLDER_OPEN`, which until now drew a flag.
            let glyph = match is_open {
                true => icons::FOLDER_OPEN,
                false => icons::FOLDER,
            };
            let resp = ui.selectable_label(is_cursor, format!("{glyph} {}", node.name));
            if is_cursor && ctx.nav.scroll_to_cursor {
                resp.scroll_to_me(Some(egui::Align::Center));
            }
            // Moving the cursor is the whole of what a click does here.
            // What that selects is derived from it once, by the panel —
            // see `AssetNav::take_cursor_move`.
            if resp.clicked() {
                ctx.nav.cursor = Some(node.path.clone());
            }
            // An entity dragged out of the World panel and dropped here is
            // saved as a prefab in this folder — the second of the two
            // ways to author one, beside the entity's own context menu.
            //
            // Guarded behind `dnd_hover_payload` because
            // `dnd_release_payload` takes the payload before checking its
            // type; see the ordering note in `panels/world/entity_row.rs`.
            if ctx.writable && resp.dnd_hover_payload::<Entity>().is_some() {
                ui.painter().rect_filled(
                    resp.rect,
                    2.0,
                    egui::Color32::from_rgba_unmultiplied(60, 200, 100, 40),
                );
                if let Some(entity) = resp.dnd_release_payload::<Entity>() {
                    ctx.actions.push(EditorAction::SavePrefab {
                        entity: *entity,
                        dest: Some(node.path.clone()),
                    });
                }
            }
            let writable = ctx.writable;
            let actions = &mut *ctx.actions;
            let rename = &mut *ctx.rename;
            let pending = &mut *ctx.pending;
            resp.context_menu(|ui| folder_menu(ui, node, writable, actions, rename, pending));
        })
        .body(|ui| render_children(ui, node, ctx, root));
}

pub(super) fn render_leaf(
    ui: &mut egui::Ui,
    leaf: &FileLeaf,
    ctx: &mut RenderCtx<'_>,
    root: &Path,
) {
    if ctx.rename.as_ref().is_some_and(|r| r.path == leaf.path) {
        rename_edit(ui, &leaf.path, false, ctx.actions, ctx.rename);
        return;
    }

    let icon = match &leaf.asset {
        Some((_, type_name)) => type_icon(type_name),
        None => file_icon(&leaf.name),
    };
    let is_cursor = ctx.nav.is_cursor(&leaf.path);
    ctx.nav.rows.push(AssetRow {
        path: leaf.path.clone(),
        is_folder: false,
        open: false,
    });

    let mut resp = ui.selectable_label(is_cursor, format!("{icon} {}", leaf.name));
    if is_cursor && ctx.nav.scroll_to_cursor {
        resp.scroll_to_me(Some(egui::Align::Center));
    }

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
        ctx.nav.cursor = Some(leaf.path.clone());
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
