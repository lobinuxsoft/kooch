//! Chasing a goal velocity, which is what makes a floating capsule stop.

use std::collections::HashMap;

use glam::Vec3;

use kooch_ecs::entity::Entity;

use crate::walk::Walk;

/// Each character's goal velocity between steps — controller state, not an authored component.
#[derive(Default)]
pub struct WalkGoals {
    goals: HashMap<Entity, Vec3>,
    seen: HashMap<Entity, Vec3>,
}

impl WalkGoals {
    /// Advances one character's goal at `acceleration` and returns it, so the stick is followed at
    /// a fixed rate.
    pub fn chase(&mut self, entity: Entity, wanted: Vec3, acceleration: f32, dt: f32) -> Vec3 {
        let goal = self.goals.entry(entity).or_insert(Vec3::ZERO);
        *goal = towards(*goal, wanted, acceleration * dt);
        *goal
    }

    /// This character's goal, for something that only wants to look.
    pub fn of(&self, entity: Entity) -> Option<Vec3> {
        self.goals.get(&entity).copied()
    }

    /// Puts a character's goal at its current velocity — every air step, so a landing does not
    /// spend a stale goal as a shove.
    pub fn hold(&mut self, entity: Entity, velocity: Vec3) {
        self.goals.insert(entity, velocity);
    }

    /// The acceleration a character actually got, from its velocity — a body shoving a wall gets
    /// full force and no speed, and leaned 29° from it.
    pub fn gained(&mut self, entity: Entity, velocity: Vec3, dt: f32) -> Vec3 {
        let last = self.seen.insert(entity, velocity).unwrap_or(velocity);
        match dt > 0.0 {
            true => (velocity - last) / dt,
            false => Vec3::ZERO,
        }
    }

    /// Drops a character that no longer exists.
    pub fn forget(&mut self, entity: Entity) {
        self.goals.remove(&entity);
        self.seen.remove(&entity);
    }

    /// How many characters are being tracked.
    pub fn len(&self) -> usize {
        self.goals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.goals.is_empty()
    }
}

/// A step of at most `limit` from `from` towards `to`, stopping there.
fn towards(from: Vec3, to: Vec3, limit: f32) -> Vec3 {
    let delta = to - from;
    let distance = delta.length();
    if distance <= limit || distance < 1e-6 {
        return to;
    }
    from + delta / distance * limit
}

/// The acceleration that would reach `goal` this step, capped — uncapped it is a teleport, and the
/// cap stops it shoving heavy crates.
pub fn needed(goal: Vec3, velocity: Vec3, max_force: f32, dt: f32) -> Vec3 {
    if dt <= 0.0 {
        return Vec3::ZERO;
    }
    let wanted = (goal - velocity) / dt;
    match wanted.length() > max_force.max(0.0) {
        true => wanted.normalize_or_zero() * max_force.max(0.0),
        false => wanted,
    }
}

/// What the character asks for in its walking plane, throttle clamped so a diagonal stick is not
/// faster.
pub fn goal(steering: Vec3, up: Vec3, walk: &Walk) -> Vec3 {
    let flat = steering - up * steering.dot(up);
    let throttle = flat.length().min(1.0);
    flat.normalize_or_zero() * walk.max_speed * throttle
}

/// Steering in the air only adds, towards the stick and never past the arrival speed — the ground
/// chase would stop a jump dead in mid-air.
pub fn drift(steering: Vec3, velocity: Vec3, up: Vec3, walk: &Walk, dt: f32) -> Vec3 {
    let flat = steering - up * steering.dot(up);
    let Some(direction) = flat.try_normalize() else {
        return Vec3::ZERO;
    };
    let control = walk.air_control.clamp(0.0, 1.0);
    let push = direction * walk.acceleration * control * flat.length().min(1.0);

    // Whatever it came in with, or the walking speed if that is more —
    // otherwise air control could not correct a jump taken standing
    // still.
    let ceiling = velocity.length().max(walk.max_speed);
    let after = velocity + push * dt;
    if after.length() <= ceiling || dt <= 0.0 {
        return push;
    }
    (after.normalize_or_zero() * ceiling - velocity) / dt
}

/// A push with the part into a wall removed; steering along it is how a character rounds a corner.
pub fn alongside(push: Vec3, wall: Option<Vec3>) -> Vec3 {
    let Some(normal) = wall.and_then(|normal| normal.try_normalize()) else {
        return push;
    };
    let into = push.dot(normal);
    match into < 0.0 {
        true => push - normal * into,
        false => push,
    }
}

#[cfg(test)]
mod tests;
