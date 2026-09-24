//! Participation, separate from [`SurfaceMaterial`](super::SurfaceMaterial): whether a pair is
//! considered, whether it pushes or only reports, and whether the engine hears (#561).

/// Memberships and filter, rapier's shape for collision and solver filtering: a pair interacts only
/// when **each** side's memberships meet the other's filter. Default: every group, everything
/// interacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionMask {
    /// Which groups this collider belongs to.
    pub memberships: u32,
    /// Which groups this collider will interact with.
    pub filter: u32,
}

impl Default for InteractionMask {
    fn default() -> Self {
        Self::ALL
    }
}

impl InteractionMask {
    /// In every group, interacting with every group.
    pub const ALL: Self = Self {
        memberships: u32::MAX,
        filter: u32::MAX,
    };

    /// In no group and interacting with nothing.
    pub const NONE: Self = Self {
        memberships: 0,
        filter: 0,
    };

    /// Rapier's rule, restated so the sync layer and Inspector can answer "will these touch"
    /// without a physics world.
    pub fn interacts_with(self, other: Self) -> bool {
        self.memberships & other.filter != 0 && other.memberships & self.filter != 0
    }
}

/// How a collider participates beyond geometry and surface. Default is rapier's: solid, silent, all
/// groups — events are opt-in per collider, so cost follows what the game listens for.
///
/// 🔴 No solver groups: under rapier's `And` rule a pair pushes only if both sides agree, so a
/// solver mask says what the project's matrix already says and can contradict it. A collider is
/// solid to everything the matrix lets it meet, or a trigger for all of them (#1309).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColliderInteraction {
    /// Which pairs are considered at all.
    pub collision_groups: InteractionMask,
    /// Report overlap, never solve. Rapier computes no manifold for it, so its events carry no
    /// contact data.
    pub is_trigger: bool,
    /// Raise an event when this collider starts or stops touching
    /// something.
    pub collision_events: bool,
    /// Raise an event above [`contact_force_threshold`](Self::contact_force_threshold) — "hit hard
    /// enough to hurt" without walking manifolds every frame.
    pub contact_force_events: bool,
    /// The force above which a contact is worth reporting.
    pub contact_force_threshold: f32,
}

impl Default for ColliderInteraction {
    fn default() -> Self {
        Self {
            collision_groups: InteractionMask::ALL,
            is_trigger: false,
            collision_events: false,
            contact_force_events: false,
            contact_force_threshold: 0.0,
        }
    }
}

impl ColliderInteraction {}

#[cfg(test)]
mod tests;
