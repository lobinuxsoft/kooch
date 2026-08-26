//! Asset Browser panel — a folder tree over the project's (and the
//! shipped engine's) `assets/` directories, Unity / Godot style.
//!
//! Selecting an asset drives the **Inspector** (which renders its import
//! settings / editable parameters — see
//! [`crate::panels::inspector::asset_view`]). Dropping OS files onto the
//! panel copies them into the selected project folder and re-imports, so
//! they appear in the tree and in the material texture pickers.
//!
//! Only *typed* entries appear — an asset with no `.meta` `asset_type`
//! (never touched by a typed `load::<T>`) is skipped upstream in
//! [`AssetCatalogEntry::collect_from_database`], because there is no
//! type to file it under.

mod tree;

pub(crate) use tree::AssetNav;

use std::path::{Path, PathBuf};

use kooch_core::Guid;

use crate::actions::EditorAction;
use crate::icons;
use crate::panels::inspector::AssetCatalogEntry;

use self::tree::{PendingCreate, RenameState, RenderCtx};

/// Content of the "Asset Browser" tab.
pub(crate) fn draw_asset_browser_content(
    ui: &mut egui::Ui,
    focused: bool,
    nav: &mut tree::AssetNav,
    catalog: &[AssetCatalogEntry],
    selected_asset: &mut Option<Guid>,
    current_folder: &mut Option<PathBuf>,
    engine_root: Option<&Path>,
    project_root: Option<&Path>,
    main_scene: Option<&Path>,
    actions: &mut Vec<EditorAction>,
) {
    // Full panel area, captured before content so drop detection can
    // test the pointer against it.
    let panel_rect = ui.available_rect_before_wrap();

    // Search box.
    let search_id = ui.id().with("asset_browser_search");
    let mut query: String = ui
        .ctx()
        .data(|d| d.get_temp::<String>(search_id))
        .unwrap_or_default();
    let search_resp = ui.add(
        egui::TextEdit::singleline(&mut query)
            .id(search_id)
            .desired_width(f32::INFINITY)
            .hint_text(format!("{} Search assets…", icons::MAGNIFYING_GLASS)),
    );
    if search_resp.changed() {
        ui.ctx()
            .data_mut(|d| d.insert_temp(search_id, query.clone()));
    }
    let needle = query.trim().to_lowercase();

    // Drag-and-drop import banner + destination.
    let import_dest = import_destination(current_folder.as_deref(), project_root);
    draw_drop_banner(ui, project_root, import_dest.as_deref());
    ui.separator();

    if catalog.is_empty() && project_root.is_none() {
        ui.weak("(no assets)");
        ui.weak("Open a project to import and manage assets.");
        return;
    }

    // Handle a file drop that landed over this panel BEFORE the tree
    // render, so `actions` stays available afterwards for the tree's own
    // context-menu emissions. `dropped_files` is global, so gate on the
    // pointer being inside the panel rect.
    let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
        i.raw
            .dropped_files
            .iter()
            .filter_map(|f| f.path.clone())
            .collect()
    });
    if !dropped.is_empty() {
        let over_panel = ui.ctx().input(|i| {
            i.pointer
                .interact_pos()
                .is_some_and(|p| panel_rect.contains(p))
        });
        if over_panel && let Some(dest) = import_dest {
            actions.push(EditorAction::ImportAssets {
                files: dropped,
                dest,
            });
        }
    }

    // Inline rename + create state live in egui memory (transient UI).
    let rename_id = egui::Id::new("asset_browser_rename");
    let pending_id = egui::Id::new("asset_browser_pending_create");
    let mut rename: Option<RenameState> = ui.ctx().data(|d| d.get_temp(rename_id));
    let mut pending: Option<PendingCreate> = ui.ctx().data(|d| d.get_temp(pending_id));

    if focused {
        handle_keyboard(ui, nav, actions);
    }

    egui::ScrollArea::vertical()
        // Width comes from the panel, not from the longest file name.
        // Left to shrink, the whole tree — rows, drop targets and all —
        // ends wherever the text does, and the rest of the panel does
        // nothing when clicked.
        .auto_shrink([false, true])
        .id_salt("asset_browser_grid")
        .show(ui, |ui| {
            let mut ctx = RenderCtx {
                needle: &needle,
                actions,
                rename: &mut rename,
                pending: &mut pending,
                writable: true,
                nav,
                has_settings: has_render_settings(catalog, project_root),
                project_root,
                main_scene,
            };
            // Cleared here and refilled as rows are drawn, so the list the
            // keyboard reads next frame is exactly what is on screen now.
            ctx.nav.rows.clear();

            if let Some(root) = project_root {
                let entries = entries_under(catalog, root);
                ctx.writable = true;
                tree::render_root(ui, "Project", root, &entries, &mut ctx);
            }
            if let Some(root) = engine_root {
                let entries = entries_under(catalog, root);
                ctx.writable = false;
                tree::render_root(ui, "Engine (read-only)", root, &entries, &mut ctx);
            }
        });

    // The single writer. The cursor was moved above by whichever hand the
    // user used, and this is where — and the only place — that becomes a
    // selection. Placed after the tree so it reads the rows just drawn
    // rather than last frame's.
    if let Some(row) = nav.take_cursor_move() {
        match row.is_folder {
            true => {
                *current_folder = Some(row.path);
                *selected_asset = None;
            }
            false => {
                // A file's folder is what new files go into, so arrowing
                // onto a file keeps the import destination somewhere that
                // exists.
                *current_folder = row.path.parent().map(Path::to_path_buf);
                *selected_asset = asset_guid_at(&row.path);
            }
        }
    }

    ui.ctx().data_mut(|d| {
        match rename {
            Some(state) => {
                d.insert_temp(rename_id, state);
            }
            None => {
                d.remove::<RenameState>(rename_id);
            }
        }
        match pending {
            Some(state) => {
                d.insert_temp(pending_id, state);
            }
            None => {
                d.remove::<PendingCreate>(pending_id);
            }
        }
    });
}

/// Resolves the drop destination: the selected folder if it lives inside
/// the project, otherwise the project assets root. `None` when no
/// project is open (imports are project-only; engine is read-only).
fn import_destination(
    current_folder: Option<&Path>,
    project_root: Option<&Path>,
) -> Option<PathBuf> {
    // 🔴 Always inside `assets/`, and the selected folder only when it
    // already is. A dropped texture used to land in whatever folder was
    // selected — or in the **project root** when none was, beside
    // `Cargo.toml`, where nothing registers it, no build carries it, and
    // the Asset Browser still lists it as though it were an asset.
    //
    // The same rule the "New …" menu enforces (#765), applied to the
    // other way a file enters a project. A rule that holds for one
    // entrance and not the other is not a rule.
    let assets = project_root?.join("assets");
    match current_folder {
        Some(dir) if dir.starts_with(&assets) => Some(dir.to_path_buf()),
        _ => Some(assets),
    }
}

/// Renders the import hint. Shows the live destination while files hover,
/// or a static prompt otherwise.
fn draw_drop_banner(ui: &mut egui::Ui, project_root: Option<&Path>, dest: Option<&Path>) {
    let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
    match (project_root, dest) {
        (Some(_), Some(dest)) => {
            let name = dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| dest.display().to_string());
            let text = format!(
                "{} Drop files to import into '{}'",
                icons::FOLDER_PLUS,
                name
            );
            if hovering {
                ui.strong(text);
            } else {
                ui.weak(text);
            }
        }
        _ => {
            ui.weak(format!(
                "{} Open a project to import assets (drag & drop)",
                icons::FOLDER_PLUS,
            ));
        }
    }
}

/// Whether the project already holds a `.rendersettings`.
///
/// By type and not by extension, because that is how the renderer finds
/// it (`apply_render_settings_system`) — an entry the catalog does not
/// type is one the renderer will not read either, and the menu has to
/// agree with what actually takes effect.
///
/// Scoped to the project: the engine ships assets too, and one of those
/// must not stop a project from authoring its own.
fn has_render_settings(catalog: &[AssetCatalogEntry], project_root: Option<&Path>) -> bool {
    let wanted = std::any::type_name::<kooch_render::settings::RenderSettings>();
    catalog.iter().any(|entry| {
        entry.type_name == wanted && project_root.is_some_and(|root| entry.path.starts_with(root))
    })
}

/// Catalog entries whose path lives under `root`.
fn entries_under<'a>(catalog: &'a [AssetCatalogEntry], root: &Path) -> Vec<&'a AssetCatalogEntry> {
    catalog
        .iter()
        .filter(|e| e.path.starts_with(root))
        .collect()
}

/// Moves the cursor through the tree, and acts on the row it lands on.
///
/// Reads the rows the renderer recorded last frame — see `tree::nav` for
/// why the list comes from there rather than from a second walk. Only
/// reached when this panel has focus (#661).
fn handle_keyboard(ui: &egui::Ui, nav: &mut tree::AssetNav, actions: &mut Vec<EditorAction>) {
    // Cleared every frame: a scroll request is for the frame after the key,
    // and leaving it set would fight the scrollbar for as long as the
    // cursor existed.
    nav.scroll_to_cursor = false;

    // A text field with keyboard focus owns the arrows. Inline rename and
    // inline create both put one in this tree, and a caret that cannot
    // move is worse than a cursor that does not.
    if ui.memory(|m| m.focused().is_some()) {
        return;
    }

    let (up, down, left, right, home, end, enter) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::ArrowLeft),
            i.key_pressed(egui::Key::ArrowRight),
            i.key_pressed(egui::Key::Home),
            i.key_pressed(egui::Key::End),
            i.key_pressed(egui::Key::Enter),
        )
    });

    if up {
        nav.step(-1);
    }
    if down {
        nav.step(1);
    }
    if right {
        nav.expand_or_enter();
    }
    if left {
        nav.collapse_or_parent();
    }
    if home {
        nav.to_edge(false);
    }
    if end {
        nav.to_edge(true);
    }

    // Enter used to *commit* the cursor — make it the create target, or
    // send it to the Inspector. Both of those now happen the moment the
    // cursor moves, so all that is left is the one thing Enter does that
    // moving a cursor does not: open the file, same as a double-click.
    // No longer conditional on a project root being known: the handler
    // resolves the workspace, and a file under the engine tree is worth
    // opening too.
    if enter
        && let Some(row) = nav.current()
        && !row.is_folder
    {
        actions.push(EditorAction::OpenInIde {
            file: row.path.clone(),
        });
    }
}

/// The registered asset at `path`, if there is one.
///
/// Read from the `.meta` beside the file rather than from the catalog:
/// the catalog is keyed by guid, and the keyboard only knows a path.
fn asset_guid_at(path: &Path) -> Option<Guid> {
    // `read_meta`, not `read_or_create`: navigating past a plain file must
    // not write a `.meta` beside it.
    kooch_core::asset_meta::read_meta(path)
        .ok()
        .map(|meta| meta.guid)
}

#[cfg(test)]
mod settings_tests;
