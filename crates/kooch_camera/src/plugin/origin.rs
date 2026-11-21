//! The rig's memory follows the world when the floating origin moves (#51): a cached position left
//! behind reads as the target having jumped a cell, and the damping swings the camera across it.

use kooch_core::resource::Resources;
use kooch_ecs::hierarchy::OriginShift;

use super::CameraBlend;
use crate::impulse::Impulses;
use crate::rig::RigMemory;

/// Shifts every world position the rig carries between frames.
pub(super) fn follow_origin(resources: &mut Resources) {
    let Some(by) = OriginShift::of(resources) else {
        return;
    };
    if let Some(memory) = resources.get_mut::<RigMemory>() {
        memory.arms.shift(by);
        memory.tracked.shift(by);
        memory.composed.shift(by);
        memory.leads.shift(by);
    }
    if let Some(impulses) = resources.get_mut::<Impulses>() {
        impulses.shift(by);
    }
    if let Some(blend) = resources.get_mut::<CameraBlend>() {
        blend.from_pos += by;
    }
}
