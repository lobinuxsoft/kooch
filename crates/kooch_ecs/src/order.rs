//! Where an entity sits among its siblings.
//!
//! # Why this has to be a component
//!
//! It was never stored. The World panel sorted by `Entity::index` and
//! [`SceneDocument`](crate::scene::SceneDocument) sorted the file the same
//! way — two independent sorts that happened to agree because both read
//! the allocator's numbering. So the order on screen was not a decision
//! anybody made; it was the order the entities happened to be created in,
//! and there was nothing to change when somebody wanted a different one.
//!
//! An index cannot be that decision either. It names a slot the allocator
//! reuses, so renumbering live entities would mean despawning and
//! respawning them — which invalidates every handle: selection, gizmos,
//! and the remote mirror's id map.
//!
//! # Why the values are spaced
//!
//! Siblings are numbered [`Order::STEP`] apart rather than 0, 1, 2, so
//! dropping one between two others writes **one** value instead of
//! renumbering the rest. That is not a micro-optimisation — it is what
//! keeps a scene diff readable. Moving one of thirty-six instances should
//! not show up as thirty-six changed fields.
//!
//! The gap is finite: each insertion between the same pair halves it, so
//! after about ten the room runs out and [`Order::between`] answers
//! `None`. The caller renumbers that sibling group and carries on — rare
//! enough to be worth the cheap case being cheap.
//!
//! # Identity is a different question
//!
//! Ordering needs no identity: it is a value on an entity, not a
//! reference to one. Entities that something *points at* carry
//! [`PersistentId`](crate::persistent_id::PersistentId), which is
//! deliberately opt-in for the reason described there.

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Sort key among an entity's siblings. Lower comes first.
///
/// Absent on entities nobody has ordered, which sort after those that
/// have one, by `Entity::index` — the behaviour from before this existed,
/// so a scene authored without it looks exactly as it did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Reflect)]
#[reflect(inspector = "hidden")]
pub struct Order {
    pub value: u32,
}

impl Component for Order {}

impl Order {
    /// Distance between consecutive siblings.
    ///
    /// 1000 leaves room for about ten insertions between the same pair
    /// before [`Self::between`] runs out, and `u32` still holds four
    /// million positions — more siblings than a scene will ever have.
    pub const STEP: u32 = 1000;

    pub const fn new(value: u32) -> Self {
        Self { value }
    }

    /// A value that sorts between `before` and `after`.
    ///
    /// `None` on either side means "the end": before the first, or after
    /// the last. `None` returned means the gap is exhausted and the
    /// caller has to renumber the sibling group.
    pub const fn between(before: Option<u32>, after: Option<u32>) -> Option<u32> {
        match (before, after) {
            (None, None) => Some(Self::STEP),
            // Before the first. Half the room below it, so repeated
            // drops at the top keep working until zero is reached.
            (None, Some(first)) => match first {
                0 => None,
                _ => Some(first / 2),
            },
            (Some(last), None) => last.checked_add(Self::STEP),
            // Adjacent values have nothing between them. Saturating
            // arithmetic would answer `last` and put the two in an order
            // that depends on the sort's stability rather than on this.
            (Some(a), Some(b)) if b > a + 1 => Some(a + (b - a) / 2),
            (Some(_), Some(_)) => None,
        }
    }

    /// Values `count` siblings apart, starting at [`Self::STEP`].
    ///
    /// What renumbering a group produces, and what a freshly loaded scene
    /// gets when nothing ordered it.
    pub fn spaced(count: usize) -> impl Iterator<Item = u32> {
        (1..=count as u32).map(|n| n.saturating_mul(Self::STEP))
    }
}

pub mod place;
pub use place::place;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "order/place_tests.rs"]
mod place_tests;
