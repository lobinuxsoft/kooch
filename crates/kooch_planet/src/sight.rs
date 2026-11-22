//! What a viewer can see of a planet: a cone around its view direction. Blocks outside every cone
//! stay coarse, and the generator meshes the nearest visible blocks first.

use glam::DVec3;

use crate::BlockKey;
use crate::select::{Planets, bounds};

/// Added to the view cone's half-angle, so turning the camera finds detail already there.
const MARGIN: f64 = 0.35;

/// A viewer's view cone in a planet's local space.
#[derive(Clone, Copy, Debug)]
pub struct Cone {
    /// Unit view direction.
    pub axis: DVec3,
    /// Cosine and sine of the half-angle, margin included.
    pub cos: f64,
    pub sin: f64,
}

impl Cone {
    /// A cone along `axis` covering a `fov_y` (radians) view of `aspect` width over height.
    pub fn new(axis: DVec3, fov_y: f64, aspect: f64) -> Self {
        let diagonal = ((fov_y * 0.5).tan() * (1.0 + aspect * aspect).sqrt()).atan();
        let half = (diagonal + MARGIN).min(std::f64::consts::PI);
        Self {
            axis: axis.normalize_or_zero(),
            cos: half.cos(),
            sin: half.sin(),
        }
    }

    /// A cone that sees everything, for a viewer with no direction.
    pub const ALL: Self = Self {
        axis: DVec3::Z,
        cos: -1.0,
        sin: 0.0,
    };

    /// Whether a sphere at `center` (relative to the apex) of `radius` reaches into the cone.
    fn holds(&self, center: DVec3, radius: f64) -> bool {
        let distance = center.length();
        if distance <= radius || self.axis == DVec3::ZERO {
            return true;
        }
        let along = center.dot(self.axis);
        let across = (distance * distance - along * along).max(0.0).sqrt();
        across * self.cos - along * self.sin <= radius
    }
}

/// Whether any of `planet`'s viewers can see block `key`.
pub(crate) fn sees(planets: &Planets, key: BlockKey) -> bool {
    let planet = key.planet as usize;
    let Some(shape) = planets.shape.get(planet) else {
        return false;
    };
    let (min, max) = bounds(shape, key);
    let center = (min + max) * 0.5;
    let radius = (max - min).length() * 0.5;
    planets
        .viewers(planet)
        .iter()
        .zip(planets.cones(planet))
        .any(|(viewer, cone)| cone.holds(center - *viewer, radius))
}

/// The order a block is meshed in, lowest first: the distance to its nearest viewer, a block out
/// of every view after all those in one.
pub(crate) fn urgency(planets: &Planets, key: BlockKey) -> f64 {
    let planet = key.planet as usize;
    let Some(shape) = planets.shape.get(planet) else {
        return f64::INFINITY;
    };
    let (min, max) = bounds(shape, key);
    let distance = planets
        .viewers(planet)
        .iter()
        .map(|viewer| (viewer.clamp(min, max) - *viewer).length())
        .fold(f64::INFINITY, f64::min);
    match sees(planets, key) {
        true => distance,
        false => distance + 1e12,
    }
}

#[cfg(test)]
mod tests;
