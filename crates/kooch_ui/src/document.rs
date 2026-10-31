//! The component: what a scene saves of a UI. The node tree it describes lives in [`UiTrees`].
//!
//! [`UiTrees`]: crate::UiTrees

use kooch_core::Guid;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::FieldChoice;

/// Drawn over the screen, after the opaque pass.
pub const MODE_OVERLAY: u32 = 0;
/// Drawn in the scene, placed by the entity's `GlobalTransform`.
pub const MODE_WORLD: u32 = 1;

/// What the Inspector offers for `mode`.
pub static MODE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Overlay",
        value: MODE_OVERLAY as i64,
    },
    FieldChoice {
        label: "World",
        value: MODE_WORLD as i64,
    },
];

/// A whole UI on one entity: its nodes are the document's, not the hierarchy's.
#[derive(Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(category = "UI")]
pub struct UiDocument {
    /// The `.kxml` structure (#280).
    #[reflect(asset = "kooch_ui::source::UiSource")]
    pub source: Option<Guid>,
    /// The `.kcss` sheet (#280).
    #[reflect(asset = "kooch_ui::style::sheet::UiStyleSheet")]
    pub style: Option<Guid>,
    /// Over the screen, or in the world at this entity.
    #[reflect(choices = MODE_CHOICES)]
    pub mode: u32,
    /// Kept but not drawn, picked or read: a menu that comes and goes keeps its tree.
    pub hidden: bool,
    /// Where the pointer is (Vector2). Empty reads the engine's default: the mouse.
    #[reflect(asset = "kooch_input::actions::action::Action")]
    pub point: Option<Guid>,
    /// Presses what the pointer is over. Default: left mouse button.
    #[reflect(asset = "kooch_input::actions::action::Action")]
    pub click: Option<Guid>,
    /// Moves focus (Vector2). Default: arrows, d-pad, left stick.
    #[reflect(asset = "kooch_input::actions::action::Action")]
    pub navigate: Option<Guid>,
    /// Presses what has focus. Default: Enter, Space, South.
    #[reflect(asset = "kooch_input::actions::action::Action")]
    pub submit: Option<Guid>,
    /// Sends `on:cancel`. Default: Escape, East.
    #[reflect(asset = "kooch_input::actions::action::Action")]
    pub cancel: Option<Guid>,
}

impl Component for UiDocument {}
