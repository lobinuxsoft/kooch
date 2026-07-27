//! Shared types: schedule stages and entity handles.

/// When a plugin's system runs, mirroring `ome_core::Stage`.
///
/// Kept separate from `ome_core::Stage` so a plugin does not link the
/// engine core to say when it wants to run. The host maps between them
/// in one place, and a parity test keeps the two in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    /// One-time initialisation at startup.
    Startup,
    /// Beginning of each frame.
    First,
    /// Input event processing.
    Input,
    /// Before the main update.
    PreUpdate,
    /// Main game logic.
    Update,
    /// After the main update.
    PostUpdate,
    /// GPU synchronisation.
    GpuSync,
    /// GPU command submission.
    Gpu,
    /// Physics simulation, fixed timestep.
    Physics,
    /// After physics, fixed timestep.
    PostPhysics,
    /// Before rendering.
    PreRender,
    /// Rendering.
    Render,
    /// After rendering.
    PostRender,
    /// End of frame.
    Last,
}

impl Stage {
    /// Every stage, in schedule order.
    ///
    /// Lets the host prove it maps all of them; a stage added here
    /// without a mapping fails the parity test rather than silently
    /// running at the wrong time.
    pub const ALL: &'static [Stage] = &[
        Stage::Startup,
        Stage::First,
        Stage::Input,
        Stage::PreUpdate,
        Stage::Update,
        Stage::PostUpdate,
        Stage::GpuSync,
        Stage::Gpu,
        Stage::Physics,
        Stage::PostPhysics,
        Stage::PreRender,
        Stage::Render,
        Stage::PostRender,
        Stage::Last,
    ];
}

/// Packs an entity index and generation into one handle.
///
/// Layout: low 32 bits index, high 32 bits generation.
///
/// # Example
/// ```
/// use ome_plugin_api::types::{pack_entity, unpack_entity};
///
/// let handle = pack_entity(42, 7);
/// assert_eq!(unpack_entity(handle), (42, 7));
/// ```
#[inline]
pub const fn pack_entity(index: u32, generation: u32) -> u64 {
    (index as u64) | ((generation as u64) << 32)
}

/// Unpacks a handle into `(index, generation)`.
#[inline]
pub const fn unpack_entity(handle: u64) -> (u32, u32) {
    (handle as u32, (handle >> 32) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let (idx, generation) = (42, 7);
        assert_eq!(
            unpack_entity(pack_entity(idx, generation)),
            (idx, generation)
        );
    }

    #[test]
    fn pack_unpack_zero() {
        assert_eq!(unpack_entity(pack_entity(0, 0)), (0, 0));
    }

    #[test]
    fn pack_unpack_max() {
        let handle = pack_entity(u32::MAX, u32::MAX);
        assert_eq!(unpack_entity(handle), (u32::MAX, u32::MAX));
    }

    #[test]
    fn pack_layout() {
        let handle = pack_entity(0xDEAD, 0xBEEF);
        assert_eq!(handle & 0xFFFF_FFFF, 0xDEAD);
        assert_eq!(handle >> 32, 0xBEEF);
    }

    #[test]
    fn all_lists_every_stage_in_order() {
        assert_eq!(Stage::ALL.len(), 14);
        assert_eq!(Stage::ALL[0], Stage::Startup);
        assert_eq!(Stage::ALL[13], Stage::Last);
        assert!(Stage::Update < Stage::Render);
    }
}
