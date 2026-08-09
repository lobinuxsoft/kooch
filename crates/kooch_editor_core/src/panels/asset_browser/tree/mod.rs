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
//!
//! Split by what each part does to the tree — build it, draw it, act on
//! it, name it, decorate it — with the shared context and the entry
//! point here because every submodule needs them.

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
///
/// Deliberately without the selection. The renderer's only job regarding
/// it is to move the cursor; what that selects is derived from the cursor
/// once, by the panel, so there is a single writer — see
/// [`AssetNav::take_cursor_move`]. Handing the selection down here is what
/// let a click and an arrow key disagree about which row was selected.
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
    ///
    /// Read from the catalog rather than the database: the menu draws
    /// from what the panel already has in hand, and reaching into
    /// `Resources` from a context menu is how a draw ends up owning a
    /// lookup that belongs to the frame.
    pub has_settings: bool,
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
                resp.context_menu(|ui| {
                    folder_menu(ui, &root, true, actions, rename, pending, has_settings)
                });
            }
        })
        .body(|ui| render_children(ui, &root, ctx, root_path));
}
