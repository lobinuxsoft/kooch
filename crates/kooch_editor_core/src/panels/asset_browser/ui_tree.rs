//! The Asset Browser's folder tree, drawn by `kooch_ui` (#280 phase 9). Built once per change of
//! the files or the search; opening a folder shows its children, it does not rebuild. What
//! `kooch_ui` has no widget for yet — menus, the name fields, drag payloads — is egui over a row.

mod markup;
mod overlay;

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

use kooch_ui::{UiNode, UiTree};

use self::markup::{Row, caret, folder_icon};
use super::tree::{AssetNav, AssetRow, PendingCreate, RenameState};
use crate::actions::EditorAction;
use crate::panels::inspector::AssetCatalogEntry;
use crate::ui_panel::UiPanel;

/// Seconds between walks of the disk for files the catalog does not list: code, notes, folders.
const RESCAN: f64 = 1.0;

/// What the shown document was built from and what it holds.
#[derive(Clone)]
struct Built {
    stamp: u64,
    rows: Arc<Vec<Row>>,
    /// Folders shown open, by row.
    open: Arc<HashSet<usize>>,
    /// Each row's nodes, found once: a name lookup walks the whole tree.
    nodes: Arc<Vec<Nodes>>,
    /// The rows drawn selected.
    selected: Arc<HashSet<usize>>,
    /// The disk as last walked, and when.
    disk: (u64, f64),
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
    /// `(label, root, writable)`: the project's and the engine's.
    pub roots: &'a [(&'a str, &'a Path, bool)],
    pub needle: &'a str,
    pub main_scene: Option<&'a Path>,
    pub project_root: Option<&'a Path>,
    pub has_settings: bool,
}

/// The inline edits the menus start, kept across frames by the panel.
pub(super) struct Edits<'a> {
    pub rename: &'a mut Option<RenameState>,
    pub pending: &'a mut Option<PendingCreate>,
}

pub(super) fn draw(
    ui: &mut egui::Ui,
    panel: &mut UiPanel,
    inputs: &Inputs<'_>,
    edits: Edits<'_>,
    nav: &mut AssetNav,
    actions: &mut Vec<EditorAction>,
) {
    let id = ui.id().with("asset_browser_ui_tree");
    let now = ui.input(|input| input.time);
    let built: Option<Built> = ui.data(|data| data.get_temp(id));
    let disk = match built.as_ref().map(|built| built.disk) {
        Some((hash, at)) if now - at < RESCAN => (hash, at),
        _ => (walked(inputs.roots), now),
    };
    let stamp = stamp(inputs, disk.0);
    let kept = built
        .as_ref()
        .filter(|built| built.stamp == stamp && panel.tree.is_some())
        .cloned();
    let mut built = match kept {
        Some(kept) => Built { disk, ..kept },
        None => rebuilt(panel, inputs, stamp, disk, built.as_ref()),
    };
    // A file being created shows its field under its folder, open.
    let creating = edits
        .pending
        .as_ref()
        .and_then(|pending| built.rows.iter().position(|row| row.path == pending.parent));
    if let Some(index) = creating {
        built = toggled(panel, built, index, true);
    }
    // The keyboard's open or close, as the egui tree took it.
    if let Some((path, open)) = nav.toggle.clone()
        && let Some(index) = built.rows.iter().position(|row| row.path == path)
    {
        nav.toggle = None;
        built = toggled(panel, built, index, open);
    }
    visible_rows(&built, nav);
    select(panel, &mut built, nav);
    let (response, gestures) = panel.show(ui);
    let at = |node: Option<UiNode>| {
        let name = panel.tree.as_ref()?.name(node?)?;
        let index = name.get(1..)?.parse::<usize>().ok()?;
        Some((name.starts_with('a'), index))
    };
    let clicked = at(gestures.clicked);
    let double_clicked = at(gestures.double_clicked);
    let hovered = at(gestures.hovered).map(|(_, index)| index);
    let pressed = at(gestures.drag_started).map(|(_, index)| index);
    let menu = at(gestures.secondary).map(|(_, index)| index);
    if let Some((arrow, index)) = clicked {
        match arrow {
            true => {
                let open = !built.open.contains(&index);
                built = toggled(panel, built, index, open);
            }
            false => {
                let modifiers = ui.input(|input| input.modifiers);
                let path = built.rows[index].path.clone();
                nav.click(path, modifiers.command, modifiers.shift);
            }
        }
    }
    if let Some((false, index)) = double_clicked {
        match built.rows[index].folder {
            true => {
                let open = !built.open.contains(&index);
                built = toggled(panel, built, index, open);
            }
            false => actions.push(super::tree::open_action(&built.rows[index].path)),
        }
    }
    let rect = |index: usize| {
        let node = built.nodes.get(index)?.row?;
        panel.screen_rect(node)
    };
    let row_at = |pos: egui::Pos2| {
        let name = panel.tree.as_ref()?.name(panel.node_at(pos)?)?;
        name.get(1..)?.parse::<usize>().ok()
    };
    let mut over = overlay::Overlay {
        rows: &built.rows,
        inputs,
        edits,
        actions,
        rect: &rect,
        row_at: &row_at,
    };
    over.tooltip(response.clone(), hovered);
    over.menu(ui, &response, menu);
    over.drag(ui, &response, pressed);
    over.prefab_drop(ui, &response);
    over.name_fields(ui, creating);
    ui.data_mut(|data| data.insert_temp(id, built));
}

/// The selected rows' class follows the selection, and the keyboard scrolls the cursor into view.
fn select(panel: &mut UiPanel, built: &mut Built, nav: &mut AssetNav) {
    nav.follow_cursor();
    let selected: HashSet<usize> = nav
        .selection
        .iter()
        .filter_map(|path| built.rows.iter().position(|row| &row.path == path))
        .collect();
    let Some(tree) = panel.tree.as_mut() else {
        return;
    };
    if selected != *built.selected {
        let changed = built
            .selected
            .difference(&selected)
            .map(|&index| (index, false))
            .chain(
                selected
                    .difference(&built.selected)
                    .map(|&index| (index, true)),
            );
        for (index, on) in changed {
            if let Some(node) = built.nodes.get(index).and_then(|nodes| nodes.row) {
                tree.set_class(node, "selected", on);
            }
        }
        built.selected = Arc::new(selected);
    }
    let cursor = nav
        .cursor
        .as_ref()
        .and_then(|cursor| built.rows.iter().position(|row| &row.path == cursor));
    if nav.scroll_to_cursor
        && let Some(node) = cursor.and_then(|index| built.nodes.get(index)?.row)
    {
        tree.reveal(node);
    }
}

/// A new document, keeping open the folders that were.
fn rebuilt(
    panel: &mut UiPanel,
    inputs: &Inputs<'_>,
    stamp: u64,
    disk: (u64, f64),
    before: Option<&Built>,
) -> Built {
    let (tree, rows) = markup::build(inputs);
    let was_open: HashSet<&Path> = before
        .map(|built| {
            built
                .open
                .iter()
                .map(|&index| built.rows[index].path.as_path())
                .collect()
        })
        .unwrap_or_default();
    // Roots start open, as a search opens every folder it reaches.
    let open: HashSet<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.folder)
        .filter(|(_, row)| {
            !inputs.needle.is_empty()
                || was_open.contains(row.path.as_path())
                || (before.is_none() && inputs.roots.iter().any(|(_, root, _)| *root == row.path))
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
        selected: Default::default(),
        disk,
    }
}

/// Changes when a file, the search or the main scene does.
fn stamp(inputs: &Inputs<'_>, disk: u64) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for entry in inputs.catalog {
        (&entry.path, entry.guid).hash(&mut hasher);
    }
    (disk, inputs.needle, inputs.main_scene).hash(&mut hasher);
    hasher.finish()
}

/// Every path under the roots, as a hash: what changes when a file no catalog lists appears.
fn walked(roots: &[(&str, &Path, bool)]) -> u64 {
    fn walk(dir: &Path, hasher: &mut impl Hasher) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = read.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for path in entries {
            path.hash(hasher);
            let name = path.file_name().map(|name| name.to_string_lossy());
            let skipped = name.is_some_and(|name| name == "target" || name.starts_with('.'));
            if path.is_dir() && !skipped {
                walk(&path, hasher);
            }
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (_, root, _) in roots {
        walk(root, &mut hasher);
    }
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
