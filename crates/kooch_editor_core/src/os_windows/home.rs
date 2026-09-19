//! Where a torn-off panel came from in the dock, so closing its window puts it back there.
//!
//! 🔴 Kept by tab rather than by node index: removing a panel that had a leaf of its own collapses
//! its split, and every index under it moves. A tab it sat beside, or one in the half it split off
//! from, is still findable after that.

use egui_dock::{DockState, Node, NodeIndex, Split, SurfaceIndex, TabIndex, Tree};

use crate::state::EditorTab;

/// A panel's place in the main dock surface.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Home {
    /// It shared a leaf: back beside `anchor`, at `index` among its tabs.
    Beside { anchor: EditorTab, index: usize },
    /// It had a leaf of its own: the half it split from is split again. `up` is how many levels
    /// that half's root sat above `anchor`'s leaf.
    Split {
        anchor: EditorTab,
        up: usize,
        side: Side,
        fraction: f32,
    },
}

/// Which side of its split a panel was on.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Side {
    Left,
    Right,
    Above,
    Below,
}

impl From<Side> for Split {
    fn from(side: Side) -> Self {
        match side {
            Side::Left => Split::Left,
            Side::Right => Split::Right,
            Side::Above => Split::Above,
            Side::Below => Split::Below,
        }
    }
}

/// Where `tab` sits now. `None` outside the main surface, or alone in it.
pub(crate) fn home_of(dock: &DockState<EditorTab>, tab: EditorTab) -> Option<Home> {
    let path = dock.find_tab(&tab)?;
    if path.surface != SurfaceIndex::main() {
        return None;
    }
    let tree = dock.main_surface();
    let tabs = tree[path.node].tabs()?;
    if let Some(anchor) = tabs.iter().find(|t| **t != tab) {
        return Some(Home::Beside {
            anchor: *anchor,
            index: path.tab.0,
        });
    }
    let parent = path.node.parent()?;
    let sibling = if path.node.is_left() {
        parent.right()
    } else {
        parent.left()
    };
    let (anchor, leaf) = first_tab_under(tree, sibling)?;
    let (horizontal, fraction) = match &tree[parent] {
        Node::Horizontal(split) => (true, split.fraction),
        Node::Vertical(split) => (false, split.fraction),
        _ => return None,
    };
    let side = match (horizontal, path.node.is_left()) {
        (true, true) => Side::Left,
        (true, false) => Side::Right,
        (false, true) => Side::Above,
        (false, false) => Side::Below,
    };
    Some(Home::Split {
        anchor,
        up: leaf.level() - sibling.level(),
        side,
        fraction,
    })
}

/// Puts `tab` back where `home` says. `false` when that place is gone — its anchor left the main
/// surface — and the caller has to put it somewhere else.
pub(crate) fn go_home(dock: &mut DockState<EditorTab>, tab: EditorTab, home: &Home) -> bool {
    let anchor = match home {
        Home::Beside { anchor, .. } | Home::Split { anchor, .. } => *anchor,
    };
    let Some(path) = dock.find_tab(&anchor) else {
        return false;
    };
    if path.surface != SurfaceIndex::main() {
        return false;
    }
    let tree = dock.main_surface_mut();
    match home {
        Home::Beside { index, .. } => {
            let Ok(leaf) = tree.leaf_mut(path.node) else {
                return false;
            };
            let index = (*index).min(leaf.tabs.len());
            leaf.insert_tab(TabIndex(index), tab);
            leaf.set_active_tab(TabIndex(index)).ok();
            true
        }
        Home::Split {
            up, side, fraction, ..
        } => {
            let mut node = path.node;
            for _ in 0..*up {
                match node.parent() {
                    Some(parent) => node = parent,
                    None => break,
                }
            }
            // The split takes the fraction as the parent held it, whichever side the new leaf is on.
            tree.split(node, (*side).into(), *fraction, Node::leaf(tab));
            true
        }
    }
}

/// The first tab under `root`, and the leaf it is in.
fn first_tab_under(tree: &Tree<EditorTab>, root: NodeIndex) -> Option<(EditorTab, NodeIndex)> {
    tree.iter().enumerate().find_map(|(index, node)| {
        let index = NodeIndex(index);
        let tab = node.tabs()?.first()?;
        descends_from(index, root).then_some((*tab, index))
    })
}

fn descends_from(mut node: NodeIndex, root: NodeIndex) -> bool {
    loop {
        if node == root {
            return true;
        }
        match node.parent() {
            Some(parent) if parent.0 >= root.0 => node = parent,
            _ => return false,
        }
    }
}
