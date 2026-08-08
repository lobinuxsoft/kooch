//! What a collider notices, what notices it, and what it reports.
//!
//! Separate from [`SurfaceMaterial`](super::SurfaceMaterial), which is
//! about the physics of a surface. This is about *participation*: whether a
//! pair is considered at all, whether it pushes or only reports, and
//! whether the engine hears about it.
//!
//! Until #561 none of it existed. `step` was handed `&()` for its event
//! handler, so nothing in the engine could learn that two things had
//! touched — physics could push objects around but could not drive
//! gameplay.

/// A membership-and-filter pair, the shape rapier uses for both collision
/// and solver filtering.
///
/// A pair interacts when each side's `memberships` intersects the other
/// side's `filter`. Both directions have to agree, which is the part that
/// catches people out: putting a projectile in a group the wall does not
/// filter for is not enough if the projectile does not filter for the
/// wall's group either.
///
/// # Default
///
/// In every group, filtering for every group — so everything interacts
/// with everything, which is what a scene does before anyone opts into
/// filtering.
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

    /// Whether two masks interact, by rapier's rule.
    ///
    /// Reimplemented here rather than asked of rapier because the sync
    /// layer and the Inspector both want to answer "will these two ever
    /// touch" without a physics world to ask — and because a rule this
    /// short is clearer stated than deferred.
    pub fn interacts_with(self, other: Self) -> bool {
        self.memberships & other.filter != 0 && other.memberships & self.filter != 0
    }
}

/// How a collider participates, beyond its geometry and its surface.
///
/// # Default
///
/// Solid, silent, and interacting with everything — rapier's own defaults.
/// Notably `ActiveEvents` starts empty there, which is why the engine heard
/// nothing until #561: events are opt-in per collider, so the cost is
/// proportional to what the game actually listens for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColliderInteraction {
    /// Which pairs are considered at all.
    pub collision_groups: InteractionMask,
    /// Which of the considered pairs actually push each other.
    ///
    /// The distinction is the whole point of having two: a projectile that
    /// should *detect* a wall without being stopped by it belongs to the
    /// wall's collision groups and not its solver groups.
    pub solver_groups: InteractionMask,
    /// Report overlap and never solve contacts — a trigger volume.
    ///
    /// A sensor is not a collider that happens to be ignored. Rapier
    /// computes no contact manifold for it at all, which is why a sensor's
    /// collision event carries no contact information.
    pub sensor: bool,
    /// Raise an event when this collider starts or stops touching
    /// something.
    pub collision_events: bool,
    /// Raise an event when contact force exceeds
    /// [`contact_force_threshold`](Self::contact_force_threshold).
    ///
    /// This is what separates "brushed against a wall" from "hit it hard
    /// enough to take damage" without walking contact manifolds every
    /// frame.
    pub contact_force_events: bool,
    /// The force above which a contact is worth reporting.
    pub contact_force_threshold: f32,
}

impl Default for ColliderInteraction {
    fn default() -> Self {
        Self {
            collision_groups: InteractionMask::ALL,
            solver_groups: InteractionMask::ALL,
            sensor: false,
            collision_events: false,
            contact_force_events: false,
            contact_force_threshold: 0.0,
        }
    }
}

impl ColliderInteraction {
    /// Whether anything about this collider asks rapier for events.
    pub fn wants_events(&self) -> bool {
        self.collision_events || self.contact_force_events
    }
}

#[cfg(test)]
mod tests;
