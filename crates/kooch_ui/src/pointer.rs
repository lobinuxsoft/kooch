//! The mouse over Overlay documents (#96 phase 6): what it is over, the clicks it lands, and keeping
//! those clicks from the game.

use kooch_core::event::Events;
use kooch_core::resource::Resources;
use kooch_ecs::query::Query;
use kooch_ecs::{Entity, ViewSize};
use kooch_input::{InputBackend, MouseButton};

use crate::layout::CANVAS_HEIGHT;
use crate::style::{STATE_ACTIVE, STATE_FOCUS, STATE_HOVER};
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
}

/// Where the pointer is in the UI. A game asks `over` before treating a click as its own.
#[derive(Debug, Clone, Copy, Default)]
pub struct UiPointer {
    /// The topmost node under the cursor that stops it.
    pub over: Option<(Entity, UiNode)>,
    /// The action node a press started on; a release over the same one is a click.
    pressed: Option<(Entity, UiNode)>,
    /// The action node last pressed, until a press lands elsewhere: what `:focus` selects.
    pub focused: Option<(Entity, UiNode)>,
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
    let Some(down) = resources
        .get::<Box<dyn InputBackend>>()
        .map(|backend| backend.is_mouse_pressed(MouseButton::Left))
    else {
        return;
    };
    let hit = hit(resources);
    let was = resources.get::<UiPointer>().copied().unwrap_or_default();
    let mut pointer = was;
    let target = hit.and_then(|(document, node)| action_node(resources, document, node));
    let mut clicked = None;
    if down && !pointer.was_down {
        pointer.focused = target;
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
    if let Some(event) = clicked.and_then(|(document, node)| click(resources, document, node)) {
        dispatch(resources, event);
    }
}

/// `:hover` and `:active` hold on the node and its ancestors, as in CSS; `:focus` on the node only.
fn show_states(resources: &mut Resources, was: &UiPointer, now: &UiPointer) {
    let Some(trees) = resources.get_mut::<UiTrees>() else {
        return;
    };
    let changes = [
        (was.over, now.over, STATE_HOVER, true),
        (was.pressed, now.pressed, STATE_ACTIVE, true),
        (was.focused, now.focused, STATE_FOCUS, false),
    ];
    for (before, after, bit, up) in changes {
        if before == after {
            continue;
        }
        for (target, on) in [(before, false), (after, true)] {
            let Some((document, node)) = target else {
                continue;
            };
            let Some(tree) = trees.get_mut(document) else {
                continue;
            };
            let mut current = Some(node);
            while let Some(node) = current {
                tree.set_state(node, bit, on);
                current = tree.parent(node).filter(|_| up);
            }
        }
    }
}

/// The topmost Overlay node under the pointer: documents last-drawn first.
fn hit(resources: &Resources) -> Option<(Entity, UiNode)> {
    let point = resources
        .get::<Box<dyn InputBackend>>()
        .and_then(|backend| backend.pointer())?;
    let physical = resources.get::<ViewSize>().copied().unwrap_or_default().0;
    let point = point * CANVAS_HEIGHT / physical.y.max(1) as f32;
    let mut overlays: Vec<Entity> = Vec::new();
    Query::<&UiDocument>::new(resources).for_each_entity(|entity, document| {
        if document.mode == MODE_OVERLAY {
            overlays.push(entity);
        }
    });
    let trees = resources.get::<UiTrees>()?;
    overlays.iter().rev().find_map(|&entity| {
        let node = trees.get(entity)?.pick(point)?;
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

fn click(resources: &Resources, document: Entity, node: UiNode) -> Option<UiEvent> {
    let action = resources.get::<UiTrees>()?.get(document)?.click(node)?;
    Some(UiEvent {
        document,
        node,
        kind: UiEventKind::Click,
        action: action.to_owned(),
    })
}

/// Sent for systems to read next frame, and run now when the project registered the action.
fn dispatch(resources: &mut Resources, event: UiEvent) {
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
