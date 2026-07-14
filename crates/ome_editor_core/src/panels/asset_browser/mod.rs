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

use std::path::{Path, PathBuf};

use ome_core::Guid;

use crate::actions::EditorAction;
use crate::icons;
use crate::panels::inspector::AssetCatalogEntry;

use self::tree::RenderCtx;

/// Content of the "Asset Browser" tab.
pub(crate) fn draw_asset_browser_content(
    ui: &mut egui::Ui,
    catalog: &[AssetCatalogEntry],
    selected_asset: &mut Option<Guid>,
    current_folder: &mut Option<PathBuf>,
    engine_root: Option<&Path>,
    project_root: Option<&Path>,
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

    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut ctx = RenderCtx {
            needle: &needle,
            selected_asset,
            current_folder,
            writable: true,
        };

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

    // Handle a drop that landed over this panel. `dropped_files` is
    // global, so gate on the pointer being inside the panel rect.
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
}

/// Resolves the drop destination: the selected folder if it lives inside
/// the project, otherwise the project assets root. `None` when no
/// project is open (imports are project-only; engine is read-only).
fn import_destination(
    current_folder: Option<&Path>,
    project_root: Option<&Path>,
) -> Option<PathBuf> {
    let project_root = project_root?;
    match current_folder {
        Some(dir) if dir.starts_with(project_root) => Some(dir.to_path_buf()),
        _ => Some(project_root.to_path_buf()),
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

/// Catalog entries whose path lives under `root`.
fn entries_under<'a>(catalog: &'a [AssetCatalogEntry], root: &Path) -> Vec<&'a AssetCatalogEntry> {
    catalog
        .iter()
        .filter(|e| e.path.starts_with(root))
        .collect()
}
