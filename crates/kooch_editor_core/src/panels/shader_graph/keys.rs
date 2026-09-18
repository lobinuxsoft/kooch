//! The graph's own keys (#1211): clipboard, delete and framing. Undo and redo are the editor's, and
//! reach the graph through its document history.

use egui::emath::TSTransform;
use egui::{Id, Key, Pos2, Rect, Vec2};
use egui_snarl::NodeId;

use crate::shader_graph::annotations::Annotations;
use crate::shader_graph::clipboard::{self, Clip};
use crate::shader_graph::{Graph, NODE_SIZE};

/// Where a duplicate lands, from its source: down and right, clear of the original's title.
const DUPLICATE_OFFSET: Vec2 = Vec2::new(40.0, 60.0);

/// One binding, for the shortcut sheet.
pub(super) struct Binding {
    pub(super) keys: &'static str,
    pub(super) does: &'static str,
}

/// Every binding the graph answers to, its own and the widget's, in the order the sheet lists them.
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        keys: "Ctrl+Z",
        does: "Undo",
    },
    Binding {
        keys: "Ctrl+Y · Ctrl+Shift+Z",
        does: "Redo",
    },
    Binding {
        keys: "Click a node",
        does: "Select it alone",
    },
    Binding {
        keys: "Shift+click a node",
        does: "Add it to the selection",
    },
    Binding {
        keys: "Ctrl+click a node",
        does: "Take it out of the selection",
    },
    Binding {
        keys: "Shift+drag the background",
        does: "Box-select",
    },
    Binding {
        keys: "Ctrl+Shift+drag the background",
        does: "Box-deselect",
    },
    Binding {
        keys: "Ctrl+A",
        does: "Select every node",
    },
    Binding {
        keys: "Escape · click the background",
        does: "Clear the selection",
    },
    Binding {
        keys: "Drag a node",
        does: "Move the selection",
    },
    Binding {
        keys: "Ctrl+C",
        does: "Copy the selected nodes, with the wires between them",
    },
    Binding {
        keys: "Ctrl+X",
        does: "Cut the selected nodes",
    },
    Binding {
        keys: "Ctrl+V",
        does: "Paste at the pointer, selected",
    },
    Binding {
        keys: "Ctrl+D",
        does: "Duplicate the selection",
    },
    Binding {
        keys: "Delete · Backspace",
        does: "Remove the selected nodes",
    },
    Binding {
        keys: "F",
        does: "Frame the selection (Fit frames everything)",
    },
    Binding {
        keys: "Ctrl+G",
        does: "Group the selection",
    },
    Binding {
        keys: "Drop a node inside a group",
        does: "It joins the group",
    },
    Binding {
        keys: "Drag a group's title",
        does: "Move the group's nodes",
    },
    Binding {
        keys: "Double-click a group's title",
        does: "Rename it in place",
    },
    Binding {
        keys: "Right-click a group's title",
        does: "Rename, recolour or ungroup",
    },
    Binding {
        keys: "Right-click a node",
        does: "Remove it, or take it out of its group",
    },
    Binding {
        keys: "Right-click the background",
        does: "Add a node or a note",
    },
    Binding {
        keys: "Right-click a note",
        does: "Edit or delete it",
    },
    Binding {
        keys: "Drag the background",
        does: "Pan",
    },
    Binding {
        keys: "Scroll",
        does: "Zoom",
    },
];

/// Acts on this frame's keys while the pointer is over the graph and nothing is being typed.
/// Returns the area F asked to frame, in graph space.
pub(super) fn handle(
    ui: &egui::Ui,
    graph: &mut Graph,
    annotations: &mut Annotations,
    snarl_id: Id,
    panel: Rect,
    to_screen: TSTransform,
) -> Option<Rect> {
    let ctx = ui.ctx();
    if ctx.text_edit_focused() || !ui.rect_contains_pointer(panel) {
        return None;
    }
    let selected = egui_snarl::ui::get_selected_nodes(snarl_id, ctx);
    let (command, keys) = ctx.input(|i| {
        let pressed = |key| i.key_pressed(key);
        (
            i.modifiers.command,
            [
                pressed(Key::C),
                pressed(Key::X),
                pressed(Key::V),
                pressed(Key::D),
                pressed(Key::Delete) || pressed(Key::Backspace),
                pressed(Key::F),
                pressed(Key::G),
                pressed(Key::A),
                pressed(Key::Escape),
            ],
        )
    });
    let [
        copy,
        cut,
        paste,
        duplicate,
        delete,
        frame,
        group,
        all,
        escape,
    ] = keys;
    let select = |nodes: Vec<NodeId>| egui_snarl::ui::set_selected_nodes(snarl_id, ctx, nodes);

    if command && (copy || cut) {
        if let Some(clip) = clipboard::copy(graph, &selected) {
            store(ctx, clip);
        }
        if cut {
            clipboard::remove(graph, &selected);
        }
    }
    if command
        && paste
        && let Some(clip) = stored(ctx)
    {
        let at = ctx
            .pointer_latest_pos()
            .map_or(Pos2::ZERO, |pos| to_screen.inverse() * pos);
        select(clipboard::paste(graph, &clip, at));
    }
    if command
        && duplicate
        && let (Some(clip), Some(corner)) = (
            clipboard::copy(graph, &selected),
            clipboard::corner(graph, &selected),
        )
    {
        select(clipboard::paste(graph, &clip, corner + DUPLICATE_OFFSET));
    }
    if !command && delete {
        clipboard::remove(graph, &selected);
    }
    if command && group {
        let members: Vec<usize> = selected.iter().map(|id| id.0).collect();
        annotations.group(&members);
    }
    if command && all {
        select(graph.node_ids().map(|(id, _)| id).collect());
    }
    if escape {
        select(Vec::new());
    }
    if !command && frame {
        return selection_bounds(graph, &selected);
    }
    None
}

/// Kept in egui's memory: shared by every graph this session opens, and never written to a file.
fn store(ctx: &egui::Context, clip: Clip) {
    ctx.data_mut(|d| d.insert_temp(clip_id(), clip));
}

fn stored(ctx: &egui::Context) -> Option<Clip> {
    ctx.data(|d| d.get_temp::<Clip>(clip_id()))
}

fn clip_id() -> Id {
    Id::new("shader_graph_clipboard")
}

/// What the selected nodes cover, or `None` with nothing selected.
fn selection_bounds(graph: &Graph, selected: &[NodeId]) -> Option<Rect> {
    let mut bounds = Rect::NOTHING;
    for &id in selected {
        if let Some(info) = graph.get_node_info(id) {
            bounds.extend_with(info.pos);
            bounds.extend_with(info.pos + NODE_SIZE);
        }
    }
    bounds.is_finite().then(|| bounds.expand(40.0))
}
