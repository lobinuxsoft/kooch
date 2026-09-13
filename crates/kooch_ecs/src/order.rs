//! Where an entity sits among its siblings.

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Sort key among an entity's siblings. Lower comes first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Reflect)]
#[reflect(inspector = "hidden")]
pub struct Order {
    pub value: u32,
}

impl Component for Order {}

impl Order {
    /// Distance between consecutive siblings.
    pub const STEP: u32 = 1000;

    pub const fn new(value: u32) -> Self {
        Self { value }
    }

    /// A value that sorts between `before` and `after`.
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
