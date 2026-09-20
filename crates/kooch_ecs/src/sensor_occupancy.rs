//! [`SensorOccupancy`] — who is inside which sensor, and how far in (#1222).
//!
//! A solver reports two frames: the one a body arrived and the one it left. *Staying* is the set
//! between them, which is what a region that does something while you are in it needs — and the
//! depth is what lets it do that thing gradually.
//!
//! Filled by the physics plugin, and read by anything: nothing in here is a physics type, so a
//! crate that must not depend on the solver can still ask.

use crate::entity::Entity;

/// One body inside one sensor, this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Occupant {
    /// The entity carrying the sensor collider.
    pub sensor: Entity,
    /// The entity inside it.
    pub body: Entity,
    /// How far the body's origin is past the sensor's surface, in metres. Zero at the surface, and
    /// [`f32::INFINITY`] where the shape has no cheap answer — inside is all the caller knows.
    pub depth: f32,
}

/// Every body inside every sensor. Rebuilt from the solver's arrivals and departures, so an entry
/// outlives the frame it arrived in.
#[derive(Debug, Clone, Default)]
pub struct SensorOccupancy {
    inside: Vec<Occupant>,
}

impl SensorOccupancy {
    /// Records an arrival, or moves one already recorded to `depth`.
    pub fn enter(&mut self, sensor: Entity, body: Entity, depth: f32) {
        match self.find(sensor, body) {
            Some(at) => self.inside[at].depth = depth,
            None => self.inside.push(Occupant {
                sensor,
                body,
                depth,
            }),
        }
    }

    /// Records a departure. A pair that was never inside is not an error: a sensor deleted while
    /// occupied leaves an arrival with no departure, and the reverse is a scene reload.
    pub fn leave(&mut self, sensor: Entity, body: Entity) {
        if let Some(at) = self.find(sensor, body) {
            self.inside.swap_remove(at);
        }
    }

    /// Drops everything about `entity`, whichever side it was on.
    pub fn forget(&mut self, entity: Entity) {
        self.inside
            .retain(|occupant| occupant.sensor != entity && occupant.body != entity);
    }

    pub fn clear(&mut self) {
        self.inside.clear();
    }

    /// The deepest body inside `sensor`, or `None` while it is empty. The deepest, because a region
    /// asked how much of it applies is asking about whoever is furthest in.
    pub fn depth_in(&self, sensor: Entity) -> Option<f32> {
        self.inside
            .iter()
            .filter(|occupant| occupant.sensor == sensor)
            .map(|occupant| occupant.depth)
            .fold(None, |best: Option<f32>, depth| {
                Some(best.map_or(depth, |best| best.max(depth)))
            })
    }

    pub fn iter(&self) -> impl Iterator<Item = &Occupant> {
        self.inside.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.inside.is_empty()
    }

    fn find(&self, sensor: Entity, body: Entity) -> Option<usize> {
        self.inside
            .iter()
            .position(|occupant| occupant.sensor == sensor && occupant.body == body)
    }
}

#[cfg(test)]
mod tests;
