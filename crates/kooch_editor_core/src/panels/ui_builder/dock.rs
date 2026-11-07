//! The UI Builder's panes as a dock of their own, inside its tab: dividers resize them, tabs move
//! them, and each clips what does not fit, as the rest of the editor does.

use std::path::PathBuf;

use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use serde::{Deserialize, Serialize};

use crate::panels::inspector::AssetCatalogEntry;
use crate::ui_builder::UiBuilder;

/// Shares of the panel the hierarchy and the Inspector start with beside the canvas.
const HIERARCHY_SHARE: f32 = 0.18;
const INSPECTOR_SHARE: f32 = 0.24;
/// Share of the left column the hierarchy starts with above the Library.
const TREE_SHARE: f32 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Pane {
    Hierarchy,
    Library,
    Canvas,
    Inspector,
}

struct Panes<'a> {
    builder: &'a mut UiBuilder,
    catalog: &'a [AssetCatalogEntry],
}

impl TabViewer for Panes<'_> {
    type Tab = Pane;

    fn id(&mut self, tab: &mut Pane) -> egui::Id {
        egui::Id::new(("ui_builder_pane", *tab as u8))
    }

    fn title(&mut self, tab: &mut Pane) -> egui::WidgetText {
        format!("{tab:?}").into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Pane) {
        match tab {
            Pane::Hierarchy => super::hierarchy::draw(ui, self.builder, self.catalog),
            Pane::Library => super::library::draw(ui, self.builder, self.catalog),
            Pane::Canvas => super::canvas(ui, self.builder, self.catalog),
            Pane::Inspector => super::inspector::draw(ui, self.builder, self.catalog),
        }
    }

    fn is_closeable(&self, _tab: &Pane) -> bool {
        false
    }

    fn allowed_in_windows(&self, _tab: &mut Pane) -> bool {
        false
    }

    /// Each pane scrolls itself, and the canvas pans.
    fn scroll_bars(&self, _tab: &Pane) -> [bool; 2] {
        [false, false]
    }
}

fn layout() -> DockState<Pane> {
    let mut dock = DockState::new(vec![Pane::Canvas]);
    let surface = dock.main_surface_mut();
    let [canvas, left] = surface.split_left(
        NodeIndex::root(),
        1.0 - HIERARCHY_SHARE,
        vec![Pane::Hierarchy],
    );
    let beside = 1.0 - INSPECTOR_SHARE / (1.0 - HIERARCHY_SHARE);
    surface.split_right(canvas, beside, vec![Pane::Inspector]);
    surface.split_below(left, TREE_SHARE, vec![Pane::Library]);
    dock
}

/// Beside the editor's own layout.
fn saved_path() -> Option<PathBuf> {
    Some(crate::layout::layout_path()?.with_file_name("ui_builder_layout.ron"))
}

/// The arrangement of the last session; anything unreadable is the default's business.
fn saved() -> Option<DockState<Pane>> {
    let text = std::fs::read_to_string(saved_path()?).ok()?;
    ron::from_str(&text)
        .inspect_err(|error| tracing::warn!(%error, "could not read the UI Builder's layout"))
        .ok()
}

fn save(text: &str) {
    let Some(path) = saved_path() else {
        return;
    };
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, text));
    if let Err(error) = written {
        tracing::warn!(file = %path.display(), %error, "could not save the UI Builder's layout");
    }
}

pub(super) fn draw(ui: &mut egui::Ui, builder: &mut UiBuilder, catalog: &[AssetCatalogEntry]) {
    // In egui's memory, not the builder: opening another file keeps the arrangement. The editor
    // does not save that memory, so the disk carries it between sessions.
    let id = egui::Id::new("ui_builder_dock");
    let written_id = id.with("written");
    let mut dock = ui
        .data_mut(|data| data.get_temp::<DockState<Pane>>(id))
        .or_else(saved)
        .unwrap_or_else(layout);
    DockArea::new(&mut dock)
        .id(id)
        .style(Style::from_egui(ui.style().as_ref()))
        .show_close_buttons(false)
        .show_add_buttons(false)
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(false)
        .show_inside(ui, &mut Panes { builder, catalog });
    // Once the pointer lets go: a dragged divider is one write, not one per frame.
    if !ui.input(|input| input.pointer.any_down())
        && let Ok(text) = ron::ser::to_string(&dock)
        && ui
            .data_mut(|data| data.get_temp::<String>(written_id))
            .as_ref()
            != Some(&text)
    {
        save(&text);
        ui.data_mut(|data| data.insert_temp(written_id, text));
    }
    ui.data_mut(|data| data.insert_temp(id, dock));
}
