//! The tree as a `.kxml` and its sheet: one row per folder and file, a folder's children in a
//! panel that opening it shows.

use std::path::PathBuf;

use kooch_core::Guid;
use kooch_ui::UiTree;
use kooch_ui::source::UiSource;
use kooch_ui::style::UiStyleSheet;

use super::Inputs;
use crate::icons;
use crate::panels::asset_browser::tree::FolderNode;
use crate::panels::inspector::AssetCatalogEntry;
use crate::ui_panel::ICON_FAMILY;

/// Points each level of folders indents by.
const INDENT: f32 = 14.0;

fn sheet() -> String {
    format!(
        "
ui {{ width: 100%; height: 100%; flex-direction: column; color: #d0d4dc; font-size: 13px; }}
.list {{ flex-grow: 1; flex-direction: column; overflow: scroll; }}
.children {{ flex-shrink: 0; flex-direction: column; }}
.row {{ flex-shrink: 0; flex-direction: row; align-items: center; gap: 4px; padding: 2px 6px; }}
.row:hover {{ background-color: rgba(255, 255, 255, 0.06); }}
.row.selected {{ background-color: #2f4f7f; }}
.arrow {{ width: 12px; font-size: 10px; font-family: \"{ICON_FAMILY}\"; color: #8a909c; }}
.icon {{ font-family: \"{ICON_FAMILY}\"; }}
"
    )
}

/// One row of the document, by its index in the names `r{i}`, `a{i}`, `i{i}` and `c{i}`.
pub(super) struct Row {
    pub path: PathBuf,
    /// As the menus show it; empty for a root, which cannot be renamed.
    pub name: String,
    pub folder: bool,
    /// Files only: the GUID and type a registered asset has.
    pub asset: Option<(Guid, String)>,
    /// Under the project, not the read-only engine.
    pub writable: bool,
    pub main: bool,
    pub icon: &'static str,
    /// Folders only: the rows inside it, as `start..end` of this list.
    pub inside: std::ops::Range<usize>,
}

pub(super) fn caret(open: bool) -> &'static str {
    match open {
        true => icons::CARET_DOWN,
        false => icons::CARET_RIGHT,
    }
}

pub(super) fn folder_icon(open: bool) -> &'static str {
    match open {
        true => icons::FOLDER_OPEN,
        false => icons::FOLDER,
    }
}

pub(super) fn build(inputs: &Inputs<'_>) -> (Option<UiTree>, Vec<Row>) {
    let mut markup = String::from("<ui><panel class=\"list\">");
    let mut rows = Vec::new();
    for &(label, root, writable) in inputs.roots {
        let entries: Vec<&AssetCatalogEntry> = inputs
            .catalog
            .iter()
            .filter(|entry| entry.path.starts_with(root))
            .collect();
        let node = FolderNode::build(root, &entries);
        folder(&mut markup, &mut rows, &node, label, 0, (writable, inputs));
    }
    markup.push_str("</panel></ui>");
    let tree = UiSource::parse(&markup)
        .inspect_err(|error| tracing::error!(%error, "the Asset Browser's tree does not parse"))
        .ok()
        .map(|source| kooch_ui::build_tree(&source, Some(&UiStyleSheet::parse(&sheet()))));
    (tree, rows)
}

fn folder(
    markup: &mut String,
    rows: &mut Vec<Row>,
    node: &FolderNode,
    label: &str,
    depth: usize,
    (writable, inputs): (bool, &Inputs<'_>),
) {
    let index = rows.len();
    rows.push(Row {
        path: node.path.clone(),
        // A root's label is not a name it can be renamed from.
        name: match depth {
            0 => String::new(),
            _ => node.name.clone(),
        },
        folder: true,
        asset: None,
        writable,
        main: false,
        icon: folder_icon(false),
        inside: 0..0,
    });
    row_start(markup, index, depth, None);
    markup.push_str(&format!(
        "<label name=\"a{index}\" class=\"arrow\" on:click=\"toggle\">{}</label>\
         <label name=\"i{index}\" class=\"icon\">{}</label><label>{}</label></panel>",
        caret(true),
        folder_icon(true),
        escape(label)
    ));
    markup.push_str(&format!("<panel name=\"c{index}\" class=\"children\">"));
    for sub in node.folders.values() {
        if sub.matches(inputs.needle) {
            folder(markup, rows, sub, &sub.name, depth + 1, (writable, inputs));
        }
    }
    for leaf in &node.files {
        if !inputs.needle.is_empty() && !leaf.name.to_lowercase().contains(inputs.needle) {
            continue;
        }
        let main = inputs.main_scene == Some(leaf.path.as_path());
        let family = crate::palette::of_asset(
            &leaf.name,
            leaf.asset.as_ref().map(|(_, type_name)| type_name.as_str()),
        );
        let icon = match (family, main) {
            (_, true) => icons::PLAY,
            (Some(family), _) => family.icon(),
            (None, _) => icons::LIST_BULLETS,
        };
        let colour = match main {
            true => Some(crate::palette::state::MAIN_SCENE),
            false => family.and_then(crate::palette::Family::colour),
        };
        let at = rows.len();
        rows.push(Row {
            path: leaf.path.clone(),
            name: leaf.name.clone(),
            folder: false,
            asset: leaf.asset.clone(),
            writable,
            main,
            icon,
            inside: 0..0,
        });
        row_start(markup, at, depth + 1, colour);
        // An empty arrow keeps the icons in one column with the folders'.
        markup.push_str(&format!(
            "<label class=\"arrow\"> </label><label class=\"icon\">{icon}</label><label>{}</label></panel>",
            escape(&leaf.name)
        ));
    }
    markup.push_str("</panel>");
    rows[index].inside = index + 1..rows.len();
}

fn row_start(markup: &mut String, index: usize, depth: usize, colour: Option<egui::Color32>) {
    let colour = colour
        .map(|c| format!(" color: #{:02x}{:02x}{:02x};", c.r(), c.g(), c.b()))
        .unwrap_or_default();
    markup.push_str(&format!(
        "<panel name=\"r{index}\" class=\"row\" on:click=\"row\" style=\"padding-left: {}px;{colour}\">",
        6.0 + depth as f32 * INDENT
    ));
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
