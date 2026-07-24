//! Execution stages for the game loop.
//!
//! Systems are organized into stages that run in a specific order each frame.
//! Some stages (Physics, PostPhysics) run multiple times per frame with fixed timestep.

/// Execution stage for systems.
///
/// Stages run in order from `Startup` to `Last`. Physics stages may run
/// multiple times per frame to maintain fixed timestep.
///
/// # Stage Order
/// ```text
/// Startup     → One-time initialization (runs once at app start)
/// First       → Beginning of frame
/// Input       → Process input events
/// PreUpdate   → Prepare for main update
/// Update      → Main game logic
/// Physics*    → Physics simulation (fixed timestep)
/// PostPhysics*→ Post-physics processing (fixed timestep)
/// PostUpdate  → Cleanup after main update (transform propagation)
/// GpuSync     → Synchronize with GPU
/// Gpu         → GPU command submission
/// PreRender   → Prepare for rendering
/// Render      → Main rendering
/// PostRender  → Post-render cleanup
/// Last        → End of frame
///
/// * = may run multiple times per frame
/// ```
///
/// The discriminants below still number `PostUpdate`..`Gpu` before the
/// fixed stages, because they double as the `BTreeMap` key that orders
/// systems *within* a run. Execution order is the list above, decided by
/// [`Schedule::run_pre_physics`] / `run_fixed_stages` / `run_post_physics`.
///
/// [`Schedule::run_pre_physics`]: crate::schedule::Schedule::run_pre_physics
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Stage {
    /// One-time initialization at application startup.
    Startup = 0,
    /// Beginning of each frame.
    First = 1,
    /// Input event processing.
    Input = 2,
    /// Preparation before main update.
    PreUpdate = 3,
    /// Main game logic.
    Update = 4,
    /// Cleanup after main update.
    PostUpdate = 5,
    /// GPU synchronization.
    GpuSync = 6,
    /// GPU command submission.
    Gpu = 7,
    /// Physics simulation (fixed timestep).
    Physics = 8,
    /// Post-physics processing (fixed timestep).
    PostPhysics = 9,
    /// Preparation before rendering.
    PreRender = 10,
    /// Main rendering.
    Render = 11,
    /// Post-render cleanup.
    PostRender = 12,
    /// End of frame cleanup.
    Last = 13,
}

impl Stage {
    /// Returns all stages in execution order.
    pub const ALL: [Stage; 14] = [
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

    /// Stages that run once per frame (non-fixed timestep).
    pub const FRAME_STAGES: [Stage; 12] = [
        Stage::First,
        Stage::Input,
        Stage::PreUpdate,
        Stage::Update,
        Stage::PostUpdate,
        Stage::GpuSync,
        Stage::Gpu,
        Stage::PreRender,
        Stage::Render,
        Stage::PostRender,
        Stage::Last,
        Stage::Startup, // Included for completeness but only runs once
    ];

    /// Stages that run with fixed timestep (may run multiple times per frame).
    pub const FIXED_STAGES: [Stage; 2] = [Stage::Physics, Stage::PostPhysics];

    /// Returns `true` if this stage uses fixed timestep.
    #[inline]
    pub const fn is_fixed(self) -> bool {
        matches!(self, Stage::Physics | Stage::PostPhysics)
    }

    /// Returns `true` if this stage runs only once at startup.
    #[inline]
    pub const fn is_startup(self) -> bool {
        matches!(self, Stage::Startup)
    }

    /// Returns the stage name as a string.
    pub const fn name(self) -> &'static str {
        match self {
            Stage::Startup => "Startup",
            Stage::First => "First",
            Stage::Input => "Input",
            Stage::PreUpdate => "PreUpdate",
            Stage::Update => "Update",
            Stage::PostUpdate => "PostUpdate",
            Stage::GpuSync => "GpuSync",
            Stage::Gpu => "Gpu",
            Stage::Physics => "Physics",
            Stage::PostPhysics => "PostPhysics",
            Stage::PreRender => "PreRender",
            Stage::Render => "Render",
            Stage::PostRender => "PostRender",
            Stage::Last => "Last",
        }
    }
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_ordering() {
        assert!(Stage::Startup < Stage::First);
        assert!(Stage::First < Stage::Update);
        assert!(Stage::Update < Stage::Physics);
        assert!(Stage::Physics < Stage::Render);
        assert!(Stage::Render < Stage::Last);
    }

    #[test]
    fn fixed_stages() {
        assert!(Stage::Physics.is_fixed());
        assert!(Stage::PostPhysics.is_fixed());
        assert!(!Stage::Update.is_fixed());
        assert!(!Stage::Render.is_fixed());
    }

    #[test]
    fn startup_stage() {
        assert!(Stage::Startup.is_startup());
        assert!(!Stage::First.is_startup());
    }

    #[test]
    fn all_stages_count() {
        assert_eq!(Stage::ALL.len(), 14);
    }

    #[test]
    fn stage_names() {
        assert_eq!(Stage::Startup.name(), "Startup");
        assert_eq!(Stage::Physics.name(), "Physics");
        assert_eq!(Stage::Last.name(), "Last");
    }

    #[test]
    fn display_impl() {
        assert_eq!(format!("{}", Stage::Update), "Update");
    }
}
