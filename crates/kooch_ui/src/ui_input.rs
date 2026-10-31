//! What the UI reads, as Input Actions like the game's: one input system (#96). A document's empty
//! action field reads the engine's default, as an empty Style reads the default sheet.

use std::sync::OnceLock;

use glam::Vec2;
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_input::InputBackend;
use kooch_input::actions::{Action, LoadedActions, evaluate};

use crate::UiDocument;

/// One document's input this frame.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct UiInput {
    /// Physical pixels, `+Y` down as the layout counts; `None` with no pointer.
    pub point: Option<Vec2>,
    pub click: bool,
    /// `+Y` up, as a stick.
    pub navigate: Vec2,
    pub submit: bool,
    pub cancel: bool,
}

struct Defaults {
    point: Action,
    click: Action,
    navigate: Action,
    submit: Action,
    cancel: Action,
}

fn defaults() -> &'static Defaults {
    static DEFAULTS: OnceLock<Defaults> = OnceLock::new();
    DEFAULTS.get_or_init(|| Defaults {
        point: parse(include_str!("actions/point.inputaction")),
        click: parse(include_str!("actions/click.inputaction")),
        navigate: parse(include_str!("actions/navigate.inputaction")),
        submit: parse(include_str!("actions/submit.inputaction")),
        cancel: parse(include_str!("actions/cancel.inputaction")),
    })
}

fn parse(text: &str) -> Action {
    ron::from_str(text).expect("the engine's UI actions parse")
}

pub(crate) fn read(resources: &Resources, document: &UiDocument) -> UiInput {
    let Some(backend) = resources.get::<Box<dyn InputBackend>>() else {
        return UiInput::default();
    };
    let backend: &dyn InputBackend = &**backend;
    let loaded = resources.get::<LoadedActions>();
    let defaults = defaults();
    let value = |field: Option<Guid>, fallback: &Action| {
        let action = field
            .and_then(|guid| loaded.and_then(|loaded| loaded.get(guid)))
            .unwrap_or(fallback);
        match loaded {
            Some(loaded) => loaded.read(action, backend),
            None => evaluate(action, backend),
        }
    };
    let point = value(document.point, &defaults.point).vector2();
    UiInput {
        // A pointer at the exact corner reads as none: the action has no "absent".
        point: (point != Vec2::ZERO).then(|| Vec2::new(point.x, -point.y)),
        click: value(document.click, &defaults.click).pressed,
        navigate: value(document.navigate, &defaults.navigate).vector2(),
        submit: value(document.submit, &defaults.submit).pressed,
        cancel: value(document.cancel, &defaults.cancel).pressed,
    }
}

/// A default that does not parse panics the first frame any game shows a document.
#[test]
fn the_default_actions_parse() {
    let defaults = defaults();
    assert_eq!(defaults.navigate.bindings.len(), 13);
    assert_eq!(defaults.cancel.bindings.len(), 2);
}
