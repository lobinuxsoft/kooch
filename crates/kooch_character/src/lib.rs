//! A character that stands on ground wherever it is: a dynamic capsule floating on a damped spring
//! over a swept sphere, oriented to the local up.
//! Steps, slopes, pushing and arbitrary gravity all follow from floating.

pub mod controller;
pub mod facing;
pub mod grounded;
pub mod jump;
pub mod plugin;
pub mod sprint;
pub mod touching;
pub mod walk;
pub mod wall_run;
pub mod wall_slide;

pub use controller::CharacterController;
pub use facing::Facing;
pub use grounded::Grounded;
pub use jump::{Jump, WallJump};
pub use plugin::{CharacterComponentsPlugin, CharacterPlugin};
pub use sprint::Sprint;
pub use touching::Touching;
pub use walk::Walk;
pub use wall_run::WallRun;
pub use wall_slide::WallSlide;
