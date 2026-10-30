//! A child that rides its parent's position and keeps its own rotation (#1672): a heading arrow on
//! a rolling ball.

use glam::Mat4;

use crate::component::Component;

/// Takes only the parent's world position. The child's `Transform` rotation, scale and offset are
/// in world axes; its own children compose under it as usual.
#[derive(Debug, Clone, Copy, Default, crate::Reflect)]
pub struct PositionOnly;

impl Component for PositionOnly {}

/// The matrix a child's local `Transform` multiplies.
pub fn parent_frame(parent_world: Mat4, position_only: bool) -> Mat4 {
    match position_only {
        true => Mat4::from_translation(parent_world.w_axis.truncate()),
        false => parent_world,
    }
}
