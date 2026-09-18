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

/// A titled frame behind nodes. Moving it moves the nodes it holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Group {
    pub title: String,
    pub rect: Rect,
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

    /// A group around `bounds`, titled for the user to rename.
    pub(crate) fn group(&mut self, bounds: Rect) {
        self.groups.push(Group {
            title: "Group".to_owned(),
            rect: bounds,
            color: GROUP_COLORS[0],
        });
    }

    pub(crate) fn note(&mut self, at: Pos2) {
        self.notes.push(Note {
            text: "Note".to_owned(),
            pos: at,
            width: NOTE_WIDTH,
        });
    }
}

/// Moves group `index` by `delta`, and every node whose corner sits inside it.
pub(crate) fn move_group(
    annotations: &mut Annotations,
    graph: &mut Graph,
    index: usize,
    delta: Vec2,
) {
    let Some(group) = annotations.groups.get_mut(index) else {
        return;
    };
    let inside: Vec<_> = graph
        .nodes_pos_ids()
        .filter(|(_, pos, _)| group.rect.contains(*pos))
        .map(|(id, ..)| id)
        .collect();
    group.rect = group.rect.translate(delta);
    for id in inside {
        if let Some(info) = graph.get_node_info_mut(id) {
            info.pos += delta;
        }
    }
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
