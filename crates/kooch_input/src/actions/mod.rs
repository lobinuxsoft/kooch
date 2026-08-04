//! Input actions as data — the model an editor can author.
//!
//! An action is a **name and a control type**, not a Rust enum. That is
//! the whole difference from [`crate::action_map::ActionMap`], which this
//! replaces: a type cannot be serialised, cannot appear in the Inspector,
//! and cannot be edited, so binding a key in a panel was impossible by
//! construction (#55).
//!
//! # Shape
//!
//! ```text
//! ActionMap  name, priority, actions
//!   └ Action   name, control type, bindings
//!       └ Binding   control path, processors, role in a composite
//! ```
//!
//! Bindings are a **flat list**, and a composite is a head entry followed
//! by its parts — the same representation Unity's `.inputactions` uses,
//! and for us it is also the DOD one: a contiguous `Vec`, no tree, no
//! boxed nodes.
//!
//! # What it borrows, and what it does not
//!
//! Taken from Unity's Input System: the device-class binding path, the
//! composite parts and their modes, the deadzone curves, and the flat
//! binding list.
//!
//! Deliberately not taken: processors in three places (here they live on
//! the binding alone), processors as parseable strings (here they are a
//! typed enum), and identifying an action by a type.

mod action;
mod action_ref;
mod asset;
mod binding;
mod component;
mod handle;
mod path;
mod plugin;
mod processor;
mod single;
mod state;

pub use action::{Action, ActionId, ActionMap, ControlType};
pub use action_ref::{ActionRef, ResolvedAction};
pub use asset::{INPUT_MAP_EXTENSION, InputMapLoader, InputMapParseError, save, to_ron};
pub use binding::{
    Binding, BothHeld, Composite, Group, PartName, Role, VectorMode, group_range, groups,
};
pub use component::InputMapSource;
pub use handle::ActionHandle;
pub use path::{ControlPath, DeviceClass};
pub use plugin::{ActionsPlugin, ActiveActionMap, InputComponentsPlugin};
pub use processor::{DEFAULT_DEADZONE_MAX, DEFAULT_DEADZONE_MIN, Processor};
pub use single::{
    INPUT_ACTION_EXTENSION, InputAction, InputActionLoader, LoadedActions, save as save_action,
    to_ron as action_to_ron,
};
pub use state::{ActionState, ActionValue, evaluate};
