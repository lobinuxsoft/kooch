//! The Asset Browser's assets drawn by `kooch_ui` (#1776), behind a toggle until the tree moves
//! over whole (#1777, #1778): a flat list that hovers, scrolls, selects and opens.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kooch_core::Guid;
use kooch_ui::source::UiSource;
use kooch_ui::style::UiStyleSheet;

use crate::actions::EditorAction;
use crate::panels::inspector::AssetCatalogEntry;
use crate::ui_panel::UiPanel;

const SHEET: &str = "
ui { width: 100%; height: 100%; flex-direction: column; color: #d0d4dc; font-size: 13px; }
.list { flex-grow: 1; flex-direction: column; overflow: scroll; }
.row { flex-shrink: 0; padding: 2px 6px; }
.row:hover { background-color: rgba(255, 255, 255, 0.06); }
.row.selected { background-color: #2f4f7f; }
";

/// What the shown document was built from, and each row's asset.
#[derive(Clone)]
struct Built {
    stamp: u64,
    rows: Arc<Vec<(Guid, PathBuf)>>,
}

pub(super) fn draw(
    ui: &mut egui::Ui,
    panel: &mut UiPanel,
    catalog: &[AssetCatalogEntry],
    roots: &[&Path],
    selected_asset: &mut Option<Guid>,
    current_folder: &mut Option<PathBuf>,
    actions: &mut Vec<EditorAction>,
) {
    let id = ui.id().with("asset_browser_ui_list");
    let mut entries: Vec<&AssetCatalogEntry> = catalog
        .iter()
        .filter(|entry| roots.iter().any(|root| entry.path.starts_with(root)))
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for entry in &entries {
        entry.path.hash(&mut hasher);
    }
    let stamp = hasher.finish();
    let built: Option<Built> = ui.data(|data| data.get_temp(id));
    let rows = match built.filter(|built| built.stamp == stamp && panel.tree.is_some()) {
        Some(built) => built.rows,
        None => {
            let rows: Arc<Vec<(Guid, PathBuf)>> = Arc::new(
                entries
                    .iter()
                    .map(|entry| (entry.guid, entry.path.clone()))
                    .collect(),
            );
            let names: Vec<String> = entries
                .iter()
                .map(|entry| shown(&entry.path, roots))
                .collect();
            if let Some(tree) = build(&names) {
                panel.set_tree(tree);
            }
            ui.data_mut(|data| {
                data.insert_temp(
                    id,
                    Built {
                        stamp,
                        rows: rows.clone(),
                    },
                )
            });
            rows
        }
    };
    if let Some(tree) = panel.tree.as_mut() {
        for (index, (guid, _)) in rows.iter().enumerate() {
            if let Some(node) = tree.query(&format!("r{index}")) {
                tree.set_class(node, "selected", *selected_asset == Some(*guid));
            }
        }
    }
    let (response, gestures) = panel.show(ui);
    let row = |node| {
        let name = panel.tree.as_ref()?.name(node)?;
        rows.get(name.strip_prefix('r')?.parse::<usize>().ok()?)
    };
    if let Some((_, path)) = gestures.hovered.and_then(row) {
        response.on_hover_text(path.display().to_string());
    }
    if let Some((guid, path)) = gestures.clicked.and_then(row) {
        *selected_asset = Some(*guid);
        *current_folder = path.parent().map(Path::to_path_buf);
    }
    if let Some((_, path)) = gestures.double_clicked.and_then(row) {
        actions.push(super::tree::open_action(path));
    }
}

/// The path under whichever root holds it, `/`-separated.
fn shown(path: &Path, roots: &[&Path]) -> String {
    let relative = roots
        .iter()
        .find_map(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    relative.to_string_lossy().replace('\\', "/")
}

fn build(names: &[String]) -> Option<kooch_ui::UiTree> {
    let mut markup = String::from("<ui><panel class=\"list\">");
    for (index, name) in names.iter().enumerate() {
        markup.push_str(&format!(
            "<panel name=\"r{index}\" class=\"row\" on:click=\"row\"><label>{}</label></panel>",
            escape(name)
        ));
    }
    markup.push_str("</panel></ui>");
    let source = UiSource::parse(&markup)
        .inspect_err(|error| tracing::error!(%error, "the Asset Browser's list does not parse"))
        .ok()?;
    Some(kooch_ui::build_tree(
        &source,
        Some(&UiStyleSheet::parse(SHEET)),
    ))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
