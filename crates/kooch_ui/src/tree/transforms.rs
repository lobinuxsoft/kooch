//! Transforms (#1789): a node and what is inside it lay out as usual and move when drawn, so only
//! the draw and the pointer know, as with a flip. No relayout: a system can set one every frame.

use glam::{Mat3, Vec2, Vec3};

use crate::{UiNode, UiTree};

/// How a node moves when drawn, about `origin`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiTransform {
    /// Degrees, clockwise.
    pub rotate: f32,
    /// Degrees about the horizontal axis: positive tips the top away.
    pub rotate_x: f32,
    /// Degrees about the vertical axis: positive turns the right side away.
    pub rotate_y: f32,
    pub scale: [f32; 2],
    /// Canvas units.
    pub translate: [f32; 2],
    /// Fractions of the node's box.
    pub origin: [f32; 2],
    /// Canvas units from the eye to the node; 0 has no perspective.
    pub perspective: f32,
}

impl UiTransform {
    /// Nothing moves.
    pub const NONE: Self = Self {
        rotate: 0.0,
        rotate_x: 0.0,
        rotate_y: 0.0,
        scale: [1.0, 1.0],
        translate: [0.0, 0.0],
        origin: [0.5, 0.5],
        perspective: 0.0,
    };

    /// In a plane seen from an eye: what lands past the eye's plane is cut by the GPU.
    fn matrix(&self, pivot: Vec2) -> Mat3 {
        let turn = Mat3::from_rotation_z(self.rotate.to_radians())
            * Mat3::from_rotation_x(self.rotate_x.to_radians())
            * Mat3::from_rotation_y(self.rotate_y.to_radians());
        let x = turn.x_axis * self.scale[0];
        let y = turn.y_axis * self.scale[1];
        let eye = match self.perspective > 0.0 {
            true => -1.0 / self.perspective,
            false => 0.0,
        };
        let plane = Mat3::from_cols(
            Vec3::new(x.x, x.y, x.z * eye),
            Vec3::new(y.x, y.y, y.z * eye),
            Vec3::Z,
        );
        Mat3::from_translation(pivot + Vec2::from(self.translate))
            * plane
            * Mat3::from_translation(-pivot)
    }
}

impl Default for UiTransform {
    fn default() -> Self {
        Self::NONE
    }
}

/// `point` through `matrix`, the perspective divided out.
pub(super) fn projected(matrix: Mat3, point: Vec2) -> Vec2 {
    let at = matrix * point.extend(1.0);
    at.truncate() / at.z
}

impl UiTree {
    /// Moves `node` and what is inside it, with no relayout.
    pub fn set_transform(&mut self, node: UiNode, transform: UiTransform) {
        if let Some(slot) = self.transform.get_mut(node.0 as usize) {
            *slot = transform;
        }
    }

    pub fn transform_of(&self, node: UiNode) -> UiTransform {
        let own = self.transform.get(node.0 as usize);
        own.copied().unwrap_or(UiTransform::NONE)
    }

    /// `node`'s transform as a matrix in canvas units; `None` when it does not move.
    pub fn turn_of(&self, node: UiNode) -> Option<Mat3> {
        let transform = self.transform_of(node);
        if transform == UiTransform::NONE {
            return None;
        }
        let rect = self.rect(node)?;
        let pivot = rect.min + rect.size * Vec2::from(transform.origin);
        Some(transform.matrix(pivot))
    }

    /// `point` back through `node`'s transform; one edge-on has no way back, and stays.
    pub(super) fn unturned(&self, node: UiNode, point: Vec2) -> Vec2 {
        match self.turn_of(node) {
            Some(turn) if turn.determinant().abs() > 1e-6 => projected(turn.inverse(), point),
            _ => point,
        }
    }
}
