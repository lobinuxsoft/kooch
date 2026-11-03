//! The Asset Browser's folder tree drawn by `kooch_ui` (#1777), behind the **UI** toggle until it
//! replaces the egui one (#1778). Built once per change of the files or the search; opening a
//! folder shows its children, it does not rebuild.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kooch_ui::source::UiSource;
use kooch_ui::style::UiStyleSheet;
use kooch_ui::{UiNode, UiTree};

use super::tree::{AssetNav, AssetRow, FolderNode, open_action};
use crate::actions::EditorAction;
use crate::icons;
use crate::panels::inspector::AssetCatalogEntry;
use crate::ui_panel::{ICON_FAMILY, UiPanel};

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
struct Row {
    path: PathBuf,
    folder: bool,
    /// Folders only: the rows inside it, as `start..end` of this list.
    inside: std::ops::Range<usize>,
}

/// What the shown document was built from and what it holds.
#[derive(Clone)]
struct Built {
    stamp: u64,
    rows: Arc<Vec<Row>>,
    /// Folders shown open, by row.
    open: Arc<HashSet<usize>>,
    /// Each row's nodes, found once: a name lookup walks the whole tree.
    nodes: Arc<Vec<Nodes>>,
    /// The row drawn selected.
    selected: Option<usize>,
}

/// A row's row, caret, icon and children nodes.
#[derive(Clone, Copy, Default)]
struct Nodes {
    row: Option<UiNode>,
    arrow: Option<UiNode>,
    icon: Option<UiNode>,
    children: Option<UiNode>,
}

fn index(tree: &UiTree, count: usize) -> Vec<Nodes> {
    let mut nodes = vec![Nodes::default(); count];
    for node in (0..tree.len() as u32).map(UiNode) {
        let Some(name) = tree.name(node) else {
            continue;
        };
        let Some(slot) = name
            .get(1..)
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|i| nodes.get_mut(i))
        else {
            continue;
        };
        match name.as_bytes()[0] {
            b'r' => slot.row = Some(node),
            b'a' => slot.arrow = Some(node),
            b'i' => slot.icon = Some(node),
            b'c' => slot.children = Some(node),
            _ => {}
        }
    }
    nodes
}

pub(super) struct Inputs<'a> {
    pub catalog: &'a [AssetCatalogEntry],
    /// `(label, root)`: the project's and the engine's.
    pub roots: &'a [(&'a str, &'a Path)],
    pub needle: &'a str,
    pub main_scene: Option<&'a Path>,
}

pub(super) fn draw(
    ui: &mut egui::Ui,
    panel: &mut UiPanel,
    inputs: &Inputs<'_>,
    nav: &mut AssetNav,
    actions: &mut Vec<EditorAction>,
) {
    let id = ui.id().with("asset_browser_ui_tree");
    let stamp = stamp(inputs);
    let built: Option<Built> = ui.data(|data| data.get_temp(id));
    let mut built = match built.filter(|built| built.stamp == stamp && panel.tree.is_some()) {
        Some(built) => built,
        None => {
            let (tree, rows) = build(inputs);
            // Roots open, as the egui tree starts; a search opens every folder it reaches.
            let open: HashSet<usize> = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.folder)
                .filter(|(_, row)| {
                    !inputs.needle.is_empty() || inputs.roots.iter().any(|(_, r)| *r == row.path)
                })
                .map(|(index, _)| index)
                .collect();
            let nodes = tree
                .as_ref()
                .map(|tree| index(tree, rows.len()))
                .unwrap_or_default();
            if let Some(tree) = tree {
                panel.set_tree(tree);
            }
            show_open(panel, &nodes, &rows, None, &open);
            Built {
                stamp,
                rows: Arc::new(rows),
                open: Arc::new(open),
                nodes: Arc::new(nodes),
                selected: None,
            }
        }
    };
    // The keyboard's open or close, as the egui tree takes it.
    if let Some((path, open)) = nav.toggle.clone()
        && let Some(index) = built.rows.iter().position(|row| row.path == path)
    {
        nav.toggle = None;
        built = toggled(panel, built, index, open);
    }
    visible_rows(&built, nav);
    let selected = nav
        .cursor
        .as_ref()
        .and_then(|cursor| built.rows.iter().position(|row| &row.path == cursor));
    if let Some(tree) = panel.tree.as_mut() {
        if selected != built.selected {
            for (index, on) in [(built.selected, false), (selected, true)] {
                if let Some(node) = index.and_then(|index| built.nodes.get(index)?.row) {
                    tree.set_class(node, "selected", on);
                }
            }
            built.selected = selected;
        }
        if nav.scroll_to_cursor
            && let Some(node) = selected.and_then(|index| built.nodes.get(index)?.row)
        {
            tree.reveal(node);
        }
    }
    let (response, gestures) = panel.show(ui);
    let at = |node: Option<UiNode>| {
        let name = panel.tree.as_ref()?.name(node?)?;
        let index = name.get(1..)?.parse::<usize>().ok()?;
        Some((name.starts_with('a'), index))
    };
    let hovered = at(gestures.hovered);
    let clicked = at(gestures.clicked);
    let double_clicked = at(gestures.double_clicked);
    if let Some((_, index)) = hovered {
        response.on_hover_text(built.rows[index].path.display().to_string());
    }
    if let Some((arrow, index)) = clicked {
        match arrow {
            true => {
                let open = !built.open.contains(&index);
                built = toggled(panel, built, index, open);
            }
            false => nav.cursor = Some(built.rows[index].path.clone()),
        }
    }
    if let Some((false, index)) = double_clicked {
        match built.rows[index].folder {
            true => {
                let open = !built.open.contains(&index);
                built = toggled(panel, built, index, open);
            }
            false => actions.push(open_action(&built.rows[index].path)),
        }
    }
    ui.data_mut(|data| data.insert_temp(id, built));
}

/// Changes when a file, the search or the main scene does.
fn stamp(inputs: &Inputs<'_>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for entry in inputs.catalog {
        entry.path.hash(&mut hasher);
    }
    inputs.needle.hash(&mut hasher);
    inputs.main_scene.hash(&mut hasher);
    hasher.finish()
}

fn toggled(panel: &mut UiPanel, built: Built, index: usize, open: bool) -> Built {
    if !built.rows[index].folder || built.open.contains(&index) == open {
        return built;
    }
    let mut next = (*built.open).clone();
    match open {
        true => next.insert(index),
        false => next.remove(&index),
    };
    show_open(panel, &built.nodes, &built.rows, Some(&built.open), &next);
    Built {
        open: Arc::new(next),
        ..built
    }
}

/// Shows or hides each folder's children, caret and icon where `was` and `now` differ; every
/// folder for a fresh tree, `was` `None`.
fn show_open(
    panel: &mut UiPanel,
    nodes: &[Nodes],
    rows: &[Row],
    was: Option<&HashSet<usize>>,
    now: &HashSet<usize>,
) {
    let Some(tree) = panel.tree.as_mut() else {
        return;
    };
    for (index, row) in rows.iter().enumerate() {
        let open = now.contains(&index);
        if !row.folder || was.is_some_and(|was| was.contains(&index) == open) {
            continue;
        }
        let Some(&nodes) = nodes.get(index) else {
            continue;
        };
        if let Some(node) = nodes.children {
            tree.set_visible(node, open);
        }
        if let Some(node) = nodes.arrow {
            tree.write_text(node, caret(open));
        }
        if let Some(node) = nodes.icon {
            tree.write_text(node, folder_icon(open));
        }
    }
}

/// The rows on screen, in order, for the keyboard: what is inside a closed folder is not.
fn visible_rows(built: &Built, nav: &mut AssetNav) {
    nav.rows.clear();
    let mut index = 0;
    while index < built.rows.len() {
        let row = &built.rows[index];
        let open = built.open.contains(&index);
        nav.rows.push(AssetRow {
            path: row.path.clone(),
            is_folder: row.folder,
            open,
        });
        index = match (row.folder, open) {
            (true, false) => row.inside.end,
            _ => index + 1,
        };
    }
}

fn caret(open: bool) -> &'static str {
    match open {
        true => icons::CARET_DOWN,
        false => icons::CARET_RIGHT,
    }
}

fn folder_icon(open: bool) -> &'static str {
    match open {
        true => icons::FOLDER_OPEN,
        false => icons::FOLDER,
    }
}

fn build(inputs: &Inputs<'_>) -> (Option<UiTree>, Vec<Row>) {
    let mut markup = String::from("<ui><panel class=\"list\">");
    let mut rows = Vec::new();
    for &(label, root) in inputs.roots {
        let entries: Vec<&AssetCatalogEntry> = inputs
            .catalog
            .iter()
            .filter(|entry| entry.path.starts_with(root))
            .collect();
        let node = FolderNode::build(root, &entries);
        folder(&mut markup, &mut rows, &node, label, 0, inputs);
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
    inputs: &Inputs<'_>,
) {
    let index = rows.len();
    rows.push(Row {
        path: node.path.clone(),
        folder: true,
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
            folder(markup, rows, sub, &sub.name, depth + 1, inputs);
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
            folder: false,
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
