//! Input actions as data an editor can author: an action is a name and a control type, not a Rust
//! enum (#55).
//!
//! ```text
//! ActionMap  name, priority, actions
//!   └ Action   name, control type, bindings
//!       └ Binding   control path, processors, role in a composite
//! ```
//!
//! Borrowed from Unity: device-class paths, composites, deadzone curves and the flat binding list —
//! not its three processor sites or string processors.

mod action;
mod binding;
mod path;
mod plugin;
mod processor;
mod single;
mod state;

pub use action::{Action, ActionId, ActionMap, ControlType};
pub use binding::{
    Binding, BothHeld, Composite, Group, PartName, Role, VectorMode, group_range, groups,
};
pub use path::{ControlPath, DeviceClass};
pub use plugin::{ActionsPlugin, InputComponentsPlugin};
pub use processor::{DEFAULT_DEADZONE_MAX, DEFAULT_DEADZONE_MIN, Processor};
pub use single::{
    INPUT_ACTION_EXTENSION, InputAction, InputActionLoader, LoadedActions, save as save_action,
    to_ron as action_to_ron,
};
pub use state::{ActionValue, evaluate};
