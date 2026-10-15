//! [`AxisMask`] — which axes a mode offers (#1469).
//!
//! 🔴 A handle that is drawn and does nothing reads as a broken gizmo. The three modes are offered
//! to everything, and a consumer that cannot use one silently drops its delta: pressing `E` on a
//! spline knot drew three rings that turned nothing. The mask is how a selection says so instead.

use crate::Axis;

/// A set of axes, as bits. Empty means the mode it belongs to is not offered at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AxisMask(u8);

impl AxisMask {
    pub const NONE: Self = Self(0);
    pub const X: Self = Self(1);
    pub const Y: Self = Self(2);
    pub const Z: Self = Self(4);
    pub const ALL: Self = Self(7);

    pub fn of(axis: Axis) -> Self {
        match axis {
            Axis::X => Self::X,
            Axis::Y => Self::Y,
            Axis::Z => Self::Z,
        }
    }

    pub fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether every axis of `wanted` is in this mask.
    ///
    /// All of them, not any: a plane handle needs both its axes to mean something, and offering it
    /// where only one does would drag along an axis the selection just said it cannot edit.
    pub fn covers(self, wanted: Self) -> bool {
        self.0 & wanted.0 == wanted.0
    }
}
