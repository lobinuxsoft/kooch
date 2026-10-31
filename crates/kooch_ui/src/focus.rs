//! Focus, and moving it with a pad or the keys (#96 phase 6): what a menu needs where there is no
//! mouse. Only a document that holds focus reads `navigate`, so a HUD never takes the stick.

use glam::Vec2;
use kooch_core::resource::Resources;
use kooch_core::time::Time;
use kooch_ecs::Entity;
use kooch_ecs::query::Query;

use crate::pointer::{dispatch, event};
use crate::style::{STATE_ACTIVE, STATE_FOCUS};
use crate::ui_input::read;
use crate::{UiDocument, UiEvent, UiEventKind, UiNode, UiTree, UiTrees};

/// Seconds a held direction waits before repeating, then between repeats: Unity's defaults.
const REPEAT_DELAY: f32 = 0.5;
const REPEAT_RATE: f32 = 0.1;

/// The node with focus, and what the pad was doing to it.
#[derive(Debug, Clone, Copy, Default)]
pub struct UiFocus {
    pub focused: Option<(Entity, UiNode)>,
    /// A press this frame asked for: a node, or none for a press on the game.
    requested: Option<Option<(Entity, UiNode)>>,
    held: Option<Vec2>,
    repeat_in: f32,
    /// The node `submit` went down on; a release with focus still there is a click.
    submitting: Option<(Entity, UiNode)>,
    submit_was: bool,
    cancel_was: bool,
}

impl UiFocus {
    pub(crate) fn request(&mut self, node: Option<(Entity, UiNode)>) {
        self.requested = Some(node);
    }
}

/// A node focus can land on: one with an action, unless `tabindex="-1"`.
pub(crate) fn focusable(tree: &UiTree, node: UiNode) -> bool {
    tree.click(node).is_some() && !tree.unfocusable.contains(&node)
}

impl UiTree {
    pub(crate) fn handler(&self, node: UiNode, kind: UiEventKind) -> Option<&str> {
        self.handlers
            .iter()
            .find(|(own, own_kind, _)| *own == node && *own_kind == kind)
            .map(|(_, _, action)| &**action)
    }
}

/// After [`pick_pointer`](crate::pick_pointer), which may have asked for focus.
pub fn navigate_focus(resources: &mut Resources) {
    let dt = resources
        .get::<Time>()
        .map_or(0.0, |time| time.unscaled_delta_secs());
    let mut focus = resources.get::<UiFocus>().copied().unwrap_or_default();
    let was = focus;
    let appeared = reveal(resources);
    match focus.requested.take() {
        Some(node) => focus.focused = node,
        None if appeared.is_some() => focus.focused = appeared,
        None => {}
    }
    if focus.focused.is_some_and(|(document, _)| {
        tree(resources, document).is_none()
            || document_of(resources, document).is_none_or(|d| d.hidden)
    }) {
        focus.focused = None;
    }
    let mut events = Vec::new();
    match focus.focused.and_then(|(document, _)| {
        document_of(resources, document).map(|d| (document, read(resources, &d)))
    }) {
        Some((document, input)) => {
            steer(resources, &mut focus, document, input.navigate, dt);
            if input.submit && !focus.submit_was {
                focus.submitting = focus.focused;
            }
            if !input.submit && focus.submit_was {
                if let Some((document, node)) = focus
                    .submitting
                    .filter(|_| focus.submitting == focus.focused)
                {
                    events.extend(event(resources, document, node, UiEventKind::Click));
                }
                focus.submitting = None;
            }
            if input.cancel
                && !focus.cancel_was
                && let Some((document, node)) = focus.focused
            {
                events.extend(cancel(resources, document, node));
            }
            focus.submit_was = input.submit;
            focus.cancel_was = input.cancel;
        }
        None => {
            focus.held = None;
            focus.submitting = None;
            focus.submit_was = false;
            focus.cancel_was = false;
        }
    }
    if was.focused != focus.focused {
        if let Some((document, node)) = was.focused {
            events.extend(event(resources, document, node, UiEventKind::Blur));
        }
        if let Some((document, node)) = focus.focused {
            events.extend(event(resources, document, node, UiEventKind::Focus));
        }
    }
    show_states(resources, &was, &focus);
    resources.insert(focus);
    for event in events {
        dispatch(resources, event);
    }
}

/// A direction fires once when pushed, then repeats while held.
fn steer(resources: &Resources, focus: &mut UiFocus, document: Entity, navigate: Vec2, dt: f32) {
    let Some(direction) = quantize(navigate) else {
        focus.held = None;
        return;
    };
    let fire = if focus.held == Some(direction) {
        focus.repeat_in -= dt;
        let due = focus.repeat_in <= 0.0;
        if due {
            focus.repeat_in += REPEAT_RATE;
        }
        due
    } else {
        focus.repeat_in = REPEAT_DELAY;
        true
    };
    focus.held = Some(direction);
    if !fire {
        return;
    }
    let Some((_, from)) = focus.focused else {
        return;
    };
    if let Some(next) = tree(resources, document).and_then(|tree| neighbour(tree, from, direction))
    {
        focus.focused = Some((document, next));
    }
}

/// Past halfway, along whichever axis is pushed further, in canvas space (`+Y` down).
fn quantize(navigate: Vec2) -> Option<Vec2> {
    if navigate.length() < 0.5 {
        return None;
    }
    Some(if navigate.x.abs() > navigate.y.abs() {
        Vec2::new(navigate.x.signum(), 0.0)
    } else {
        Vec2::new(0.0, -navigate.y.signum())
    })
}

/// The focusable node `direction` points at: ahead of `from`, nearest, off-axis distance counting
/// double so a button straight below beats a closer one diagonal to it.
fn neighbour(tree: &UiTree, from: UiNode, direction: Vec2) -> Option<UiNode> {
    let centre = |node: UiNode| tree.rect(node).map(|rect| rect.min + rect.size * 0.5);
    let origin = centre(from)?;
    (0..tree.len() as u32)
        .map(UiNode)
        .filter(|&node| node != from && focusable(tree, node))
        .filter_map(|node| {
            let offset = centre(node)? - origin;
            let along = offset.dot(direction);
            (along > 0.0).then(|| (node, along + (offset - direction * along).length() * 2.0))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(node, _)| node)
}

/// The focused node's or its nearest ancestor's `on:cancel`: a `<ui on:cancel>` answers for all.
fn cancel(resources: &Resources, document: Entity, node: UiNode) -> Option<UiEvent> {
    let tree = tree(resources, document)?;
    let mut current = Some(node);
    while let Some(node) = current {
        if tree.handler(node, UiEventKind::Cancel).is_some() {
            return event(resources, document, node, UiEventKind::Cancel);
        }
        current = tree.parent(node);
    }
    None
}

/// The `autofocus` of a document that just appeared, built or unhidden: opening a menu focuses it.
/// Only then: a press on the game drops focus, and it stays dropped.
fn reveal(resources: &mut Resources) -> Option<(Entity, UiNode)> {
    let mut documents = Vec::new();
    Query::<&UiDocument>::new(resources)
        .for_each_entity(|entity, document| documents.push((entity, document.hidden)));
    let trees = resources.get_mut::<UiTrees>()?;
    let mut found = None;
    for (entity, hidden) in documents {
        let Some(tree) = trees.get_mut(entity) else {
            continue;
        };
        if !hidden && !tree.shown {
            found = found.or(tree.autofocus.map(|node| (entity, node)));
        }
        tree.shown = !hidden;
    }
    found
}

fn show_states(resources: &mut Resources, was: &UiFocus, now: &UiFocus) {
    let Some(trees) = resources.get_mut::<UiTrees>() else {
        return;
    };
    let changes = [
        (was.focused, now.focused, STATE_FOCUS),
        (was.submitting, now.submitting, STATE_ACTIVE),
    ];
    for (before, after, bit) in changes {
        if before == after {
            continue;
        }
        for (target, on) in [(before, false), (after, true)] {
            if let Some((document, node)) = target
                && let Some(tree) = trees.get_mut(document)
            {
                tree.set_state(node, bit, on);
            }
        }
    }
}

fn tree(resources: &Resources, document: Entity) -> Option<&UiTree> {
    resources.get::<UiTrees>()?.get(document)
}

fn document_of(resources: &Resources, entity: Entity) -> Option<UiDocument> {
    let mut found = None;
    Query::<&UiDocument>::new(resources).for_each_entity(|own, document| {
        if own == entity {
            found = Some(document.clone());
        }
    });
    found
}
