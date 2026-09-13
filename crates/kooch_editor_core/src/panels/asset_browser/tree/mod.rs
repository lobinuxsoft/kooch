//! Folder-tree model + rendering for the Asset Browser.

mod menus;
mod model;
mod naming;
mod nav;
mod render;
mod visuals;

use std::path::Path;

use egui::collapsing_header::CollapsingState;

use crate::icons;

use crate::actions::EditorAction;
use crate::panels::inspector::AssetCatalogEntry;

use menus::folder_menu;
use render::render_children;

pub(in crate::panels) use model::{FolderNode, PendingCreate, RenameState};
pub(crate) use nav::AssetNav;

/// What drawing the tree needs.
pub(crate) struct RenderCtx<'a> {
    pub needle: &'a str,
    pub actions: &'a mut Vec<EditorAction>,
    pub rename: &'a mut Option<RenameState>,
    pub pending: &'a mut Option<PendingCreate>,
    /// `true` for the project root (writable: menus + folder targeting),
    /// `false` for the read-only engine root.
    pub writable: bool,
    /// Keyboard state. The renderer records the rows it draws here, and
    /// applies whatever the keyboard asked for on the way past.
    pub nav: &'a mut AssetNav,
    /// Whether the project already has a `.rendersettings`.
    pub has_settings: bool,
    /// The open project's root, for deciding whether a folder is one the
    /// editor scans. `None` when no project is open, which makes every
    /// folder [`FolderRole::Other`].
    pub project_root: Option<&'a Path>,
    /// The scene the project opens with, absolute (#808).
    pub main_scene: Option<&'a Path>,
}

/// Renders one source root as a top-level collapsible node. `root_path`
/// doubles as the IDE workspace for files under it.
pub(crate) fn render_root(
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
            let resp =
                crate::widgets::SelectableRow::new(format!("{} {}", icons::FOLDER_OPEN, label))
                    .show(ui);
            if ctx.writable {
                let actions = &mut *ctx.actions;
                let rename = &mut *ctx.rename;
                let pending = &mut *ctx.pending;
                let has_settings = ctx.has_settings;
                let role = model::FolderRole::of(&root.path, ctx.project_root);
                resp.context_menu(|ui| {
                    folder_menu(
                        ui,
                        &root,
                        true,
                        actions,
                        rename,
                        pending,
                        has_settings,
                        role,
                    )
                });
            }
        })
        .body(|ui| render_children(ui, &root, ctx, root_path));
}

#[cfg(test)]
mod role_tests;
