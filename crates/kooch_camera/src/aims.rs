//! The aims that need no settings: Cinemachine's `HardLookAt`, `RotateWithFollowTarget` and our
//! `PanTilt` (#1397).
//!
//! 🔴 An aim is **a component**, for the reason a body is: `VirtualCamera.look_at` stated a second
//! time what the presence of a component already states, and two statements of one fact is the
//! defect this rig has paid for seven times.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Points straight at the target — Cinemachine's `HardLookAt`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Camera")]
pub struct HardLookAt;

impl Component for HardLookAt {}

/// Copies the target's rotation — Cinemachine's `RotateWithFollowTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Camera")]
pub struct RotateWithFollowTarget;

impl Component for RotateWithFollowTarget {}

/// Looks along the spring arm: the orbit's own direction, so a shoulder offset stays off centre
/// instead of being turned back into the middle.
///
/// Cinemachine's `PanTilt` reads its own input axes; ours reads the arm the
/// [`CameraOrbit`](crate::orbit::CameraOrbit) already turns, because our arm bodies take their
/// angles from the vcam rather than from the target's rotation (#1380).
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Camera")]
pub struct PanTilt;

impl Component for PanTilt {}
