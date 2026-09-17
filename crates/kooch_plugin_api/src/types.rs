//! Shared types: schedule stages and entity handles.

/// When a plugin's system runs, mirroring `kooch_core::Stage` so a plugin need not link the engine
/// core; a parity test keeps them in step.
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
    /// Every stage in schedule order, so the host's parity test fails on an unmapped stage.
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

/// Packs an entity index (low 32 bits) and generation (high 32 bits) into one handle.
///
/// # Example
/// ```
/// use kooch_plugin_api::types::{pack_entity, unpack_entity};
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
mod tests;

/// Where a plugin's system runs inside its stage, by system name (#392). Mirrors
/// `kooch_core::schedule::Order`; a name nothing answers to is dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Order {
    pub before: Vec<String>,
    pub after: Vec<String>,
}

impl Order {
    pub fn before(name: impl Into<String>) -> Self {
        Self::default().and_before(name)
    }

    pub fn after(name: impl Into<String>) -> Self {
        Self::default().and_after(name)
    }

    pub fn and_before(mut self, name: impl Into<String>) -> Self {
        self.before.push(name.into());
        self
    }

    pub fn and_after(mut self, name: impl Into<String>) -> Self {
        self.after.push(name.into());
        self
    }
}
