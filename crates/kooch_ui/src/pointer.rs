//! The pointer over Overlay documents (#96 phase 6): what it is over, the clicks it lands, and
//! keeping those clicks from the game. Read through each document's `point` and `click` actions.

use kooch_core::event::Events;
use kooch_core::resource::Resources;
use kooch_ecs::query::Query;
use kooch_ecs::{Entity, ViewSize};
use kooch_input::InputBackend;

use crate::focus::{UiFocus, focusable};
use crate::layout::CANVAS_HEIGHT;
use crate::style::{STATE_ACTIVE, STATE_HOVER};
use crate::ui_input::{UiInput, read};
use crate::{MODE_OVERLAY, UiDocument, UiNode, UiTrees};

/// Something the UI did, named by the action the file gave it.
#[derive(Debug, Clone, PartialEq)]
pub struct UiEvent {
    pub document: Entity,
    pub node: UiNode,
    pub kind: UiEventKind,
    pub action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEventKind {
    Click,
    Focus,
    Blur,
    Cancel,
}

/// Where the pointer is in the UI. A game asks `over` before treating a click as its own.
#[derive(Debug, Clone, Copy, Default)]
pub struct UiPointer {
    /// The topmost node under the cursor that stops it.
    pub over: Option<(Entity, UiNode)>,
    /// The action node a press started on; a release over the same one is a click.
    pressed: Option<(Entity, UiNode)>,
    /// A press that started over the game stays the game's until released, even across the UI.
    game_held: bool,
    was_down: bool,
}

/// A handler the UI runs by action name, registered with [`UiActions::add`].
pub type UiAction = fn(&mut Resources, &UiEvent);

/// Actions the project runs by name instead of reading [`UiEvent`]s.
#[derive(Default)]
pub struct UiActions {
    handlers: Vec<(Box<str>, UiAction)>,
}

impl UiActions {
    pub fn add(&mut self, name: &str, handler: UiAction) {
        self.handlers.push((name.into(), handler));
    }
}

/// Runs in `PreUpdate`, after input is pumped and before anything reads actions.
pub fn pick_pointer(resources: &mut Resources) {
    // Last frame's claim would hide the UI's own click from it.
    if let Some(backend) = resources.get_mut::<Box<dyn InputBackend>>() {
        backend.claim_pointer(false);
    }
    let overlays = overlays(resources);
    let down = overlays.iter().any(|(_, input)| input.click);
    let hit = hit(resources, &overlays);
    let was = resources.get::<UiPointer>().copied().unwrap_or_default();
    let mut pointer = was;
    let target = hit.and_then(|(document, node)| action_node(resources, document, node));
    let mut clicked = None;
    if down && !pointer.was_down {
        let focused = target.filter(|&(document, node)| {
            resources
                .get::<UiTrees>()
                .and_then(|trees| trees.get(document))
                .is_some_and(|tree| focusable(tree, node))
        });
        if let Some(focus) = resources.get_mut::<UiFocus>() {
            focus.request(focused);
        }
        match target {
            Some(target) => pointer.pressed = Some(target),
            None => pointer.game_held = hit.is_none(),
        }
    }
    if !down && pointer.was_down {
        if pointer.pressed.is_some() && pointer.pressed == target {
            clicked = target;
        }
        pointer.pressed = None;
        pointer.game_held = false;
    }
    pointer.was_down = down;
    pointer.over = hit;
    let claimed = (hit.is_some() && !pointer.game_held) || pointer.pressed.is_some();
    if let Some(backend) = resources.get_mut::<Box<dyn InputBackend>>() {
        backend.claim_pointer(claimed);
    }
    resources.insert(pointer);
    show_states(resources, &was, &pointer);
    if let Some(event) =
        clicked.and_then(|(document, node)| event(resources, document, node, UiEventKind::Click))
    {
        dispatch(resources, event);
    }
}

/// Every Overlay document and what its actions read, in spawn order.
fn overlays(resources: &Resources) -> Vec<(Entity, UiInput)> {
    let mut documents = Vec::new();
    Query::<&UiDocument>::new(resources).for_each_entity(|entity, document| {
        if document.mode == MODE_OVERLAY {
            documents.push((entity, document.clone()));
        }
    });
    documents
        .into_iter()
        .map(|(entity, document)| (entity, read(resources, &document)))
        .collect()
}

/// `:hover` and `:active` hold on the node and its ancestors, as in CSS.
fn show_states(resources: &mut Resources, was: &UiPointer, now: &UiPointer) {
    let Some(trees) = resources.get_mut::<UiTrees>() else {
        return;
    };
    let changes = [
        (was.over, now.over, STATE_HOVER),
        (was.pressed, now.pressed, STATE_ACTIVE),
    ];
    for (before, after, bit) in changes {
        if before == after {
            continue;
        }
        for (target, on) in [(before, false), (after, true)] {
            let Some((document, node)) = target else {
                continue;
            };
            if let Some(tree) = trees.get_mut(document) {
                tree.set_state_up(node, bit, on);
            }
        }
    }
}

/// The topmost Overlay node under its document's pointer: documents last-drawn first.
fn hit(resources: &Resources, overlays: &[(Entity, UiInput)]) -> Option<(Entity, UiNode)> {
    let physical = resources.get::<ViewSize>().copied().unwrap_or_default().0;
    let to_canvas = CANVAS_HEIGHT / physical.y.max(1) as f32;
    let trees = resources.get::<UiTrees>()?;
    overlays.iter().rev().find_map(|&(entity, input)| {
        let node = trees.get(entity)?.pick(input.point? * to_canvas)?;
        Some((entity, node))
    })
}

/// The node itself or its nearest ancestor with an action: a click on a button's label is the
/// button's.
fn action_node(resources: &Resources, document: Entity, node: UiNode) -> Option<(Entity, UiNode)> {
    let tree = resources.get::<UiTrees>()?.get(document)?;
    let mut current = Some(node);
    while let Some(node) = current {
        if tree.click(node).is_some() {
            return Some((document, node));
        }
        current = tree.parent(node);
    }
    None
}

/// An event of `kind` on `node`, when the file gave it that handler.
pub(crate) fn event(
    resources: &Resources,
    document: Entity,
    node: UiNode,
    kind: UiEventKind,
) -> Option<UiEvent> {
    let tree = resources.get::<UiTrees>()?.get(document)?;
    let action = match kind {
        UiEventKind::Click => tree.click(node)?,
        _ => tree.handler(node, kind)?,
    };
    Some(UiEvent {
        document,
        node,
        kind,
        action: action.to_owned(),
    })
}

/// Sent for systems to read next frame, and run now when the project registered the action.
pub(crate) fn dispatch(resources: &mut Resources, event: UiEvent) {
    let handlers: Vec<UiAction> = resources
        .get::<UiActions>()
        .map(|actions| {
            actions
                .handlers
                .iter()
                .filter(|(name, _)| **name == *event.action)
                .map(|(_, handler)| *handler)
                .collect()
        })
        .unwrap_or_default();
    for handler in handlers {
        handler(resources, &event);
    }
    if let Some(events) = resources.get_mut::<Events<UiEvent>>() {
        events.send(event);
    }
}
