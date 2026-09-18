//! Groups and notes on the canvas (#1211): how a graph is organised for the person reading it.
//! Nothing here reaches the WGSL.
//!
//! 🔴 Carried in a block of their own at the end of the `.shader`, not inside `KOOCH_GRAPH`: a file
//! written before these existed still opens, and a graph without any writes the same file as
//! before.

use egui::{Pos2, Rect, Vec2};
use serde::{Deserialize, Serialize};

use super::Graph;

const MARKER: &str = "/*KOOCH_NOTES;";

/// The canvas's groups and notes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Annotations {
    pub groups: Vec<Group>,
    pub notes: Vec<Note>,
}

/// A titled frame around the nodes that belong to it. It fits them every frame, so it follows
/// them as they move; moving it moves them. Membership is explicit: a node joins by being dropped
/// inside, or by being grouped with Ctrl+G — never by a frame passing over it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Group {
    pub title: String,
    /// Node ids, as the graph numbers them.
    pub members: Vec<usize>,
    pub color: [u8; 3],
}

/// Free text on the canvas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Note {
    pub text: String,
    pub pos: Pos2,
    pub width: f32,
}

/// The colours a group can take, dark enough for light text on its title bar.
pub(crate) const GROUP_COLORS: [[u8; 3]; 6] = [
    [0x4F, 0x5B, 0x66],
    [0x3A, 0x5A, 0x8C],
    [0x2E, 0x6B, 0x45],
    [0x6B, 0x5B, 0x2E],
    [0x7A, 0x4A, 0x2A],
    [0x6B, 0x3A, 0x6B],
];

/// How wide a new note starts.
pub(crate) const NOTE_WIDTH: f32 = 220.0;

impl Annotations {
    pub(crate) fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.notes.is_empty()
    }

    /// A group of `members`, titled for the user to rename. A node is in one group at most, so they
    /// leave the ones they were in.
    pub(crate) fn group(&mut self, members: &[usize]) {
        if members.is_empty() {
            return;
        }
        self.leave(members);
        self.groups.push(Group {
            title: "Group".to_owned(),
            members: members.to_vec(),
            color: GROUP_COLORS[0],
        });
    }

    /// Takes `nodes` out of whatever group holds them. A group left empty goes with them.
    pub(crate) fn leave(&mut self, nodes: &[usize]) {
        for group in &mut self.groups {
            group.members.retain(|member| !nodes.contains(member));
        }
        self.groups.retain(|group| !group.members.is_empty());
    }

    /// Puts `nodes` in group `index`, out of any other.
    pub(crate) fn join(&mut self, index: usize, nodes: &[usize]) {
        if index >= self.groups.len() {
            return;
        }
        for (at, group) in self.groups.iter_mut().enumerate() {
            if at == index {
                for node in nodes {
                    if !group.members.contains(node) {
                        group.members.push(*node);
                    }
                }
            } else {
                group.members.retain(|member| !nodes.contains(member));
            }
        }
        self.groups.retain(|group| !group.members.is_empty());
    }

    /// Drops members the graph no longer has — removed nodes — and the groups left empty.
    pub(crate) fn prune(&mut self, graph: &Graph) {
        for group in &mut self.groups {
            group
                .members
                .retain(|&member| graph.get_node(egui_snarl::NodeId(member)).is_some());
        }
        self.groups.retain(|group| !group.members.is_empty());
    }

    pub(crate) fn note(&mut self, at: Pos2) {
        self.notes.push(Note {
            text: "Note".to_owned(),
            pos: at,
            width: NOTE_WIDTH,
        });
    }
}

/// Moves the members of group `index` by `delta`.
pub(crate) fn move_group(annotations: &Annotations, graph: &mut Graph, index: usize, delta: Vec2) {
    let Some(group) = annotations.groups.get(index) else {
        return;
    };
    for &member in &group.members {
        if let Some(info) = graph.get_node_info_mut(egui_snarl::NodeId(member)) {
            info.pos += delta;
        }
    }
}

/// The frame around group `index`'s members, from each node's drawn rect in `rects` — or its
/// position and a typical size, for a node not drawn yet. Room for the title above, and margin.
pub(crate) fn fit(
    group: &Group,
    graph: &Graph,
    rects: &std::collections::HashMap<usize, Rect>,
    header: f32,
) -> Option<Rect> {
    let mut bounds = Rect::NOTHING;
    for &member in &group.members {
        let rect = rects.get(&member).copied().or_else(|| {
            let pos = graph.get_node_info(egui_snarl::NodeId(member))?.pos;
            Some(Rect::from_min_size(pos, super::NODE_SIZE))
        });
        if let Some(rect) = rect {
            bounds = bounds.union(rect);
        }
    }
    const MARGIN: f32 = 16.0;
    bounds.is_finite().then(|| {
        Rect::from_min_max(
            bounds.min - Vec2::new(MARGIN, MARGIN + header),
            bounds.max + Vec2::splat(MARGIN),
        )
    })
}

/// `source` with the annotations appended, or unchanged when there are none.
pub(crate) fn embed(source: String, annotations: &Annotations) -> Result<String, String> {
    if annotations.is_empty() {
        return Ok(source);
    }
    let ron = ron::ser::to_string(annotations).map_err(|e| e.to_string())?;
    // 🔴 A note is free text, and `*/` in it would close the comment early.
    let ron = ron.replace("*/", "*\\u{2f}");
    Ok(format!("{}\n{MARKER}{ron}*/\n", source.trim_end()))
}

/// The annotations a `.shader` carries; none for a file without the block or with a broken one.
pub(crate) fn extract(source: &str) -> Annotations {
    let Some(start) = source.find(MARKER).map(|at| at + MARKER.len()) else {
        return Annotations::default();
    };
    let Some(end) = source[start..].find("*/").map(|at| start + at) else {
        return Annotations::default();
    };
    ron::from_str(&source[start..end]).unwrap_or_else(|error| {
        tracing::warn!(%error, "a shader carries groups and notes this editor cannot read");
        Annotations::default()
    })
}

#[cfg(test)]
mod tests;
