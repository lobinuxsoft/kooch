//! Chasing a goal velocity, which is what makes a floating capsule stop.

use std::collections::HashMap;

use glam::Vec3;

use kooch_ecs::entity::Entity;

use crate::walk::Walk;

/// Each character's goal velocity, carried between steps.
///
/// State rather than a component: it is the controller's working
/// memory, not something a scene authors, and a serialised field nobody
/// should edit is one somebody will.
#[derive(Default)]
pub struct WalkGoals {
    goals: HashMap<Entity, Vec3>,
}

impl WalkGoals {
    /// Advances one character's goal and returns it.
    ///
    /// The goal moves at `acceleration`, so the stick is followed at a
    /// fixed rate rather than instantly. Whatever chases the goal then
    /// has its own ceiling.
    pub fn chase(&mut self, entity: Entity, wanted: Vec3, acceleration: f32, dt: f32) -> Vec3 {
        let goal = self.goals.entry(entity).or_insert(Vec3::ZERO);
        *goal = towards(*goal, wanted, acceleration * dt);
        *goal
    }

    /// This character's goal, for something that only wants to look.
    pub fn of(&self, entity: Entity) -> Option<Vec3> {
        self.goals.get(&entity).copied()
    }

    /// Drops a character that no longer exists.
    pub fn forget(&mut self, entity: Entity) {
        self.goals.remove(&entity);
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

/// The acceleration that would take `velocity` to `goal` this step,
/// capped.
///
/// Uncapped this is a teleport: `(goal - velocity) / dt` is whatever it
/// takes, and at 60 Hz that is 60× the shortfall. The cap is what makes
/// it a character rather than a constraint, and what stops it shoving a
/// heavy crate across the room.
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

/// What the character is asking for, in the plane it walks in.
///
/// The throttle is the length of the steering, clamped: a stick pushed
/// past its own corner must not walk faster diagonally.
pub fn goal(steering: Vec3, up: Vec3, walk: &Walk) -> Vec3 {
    let flat = steering - up * steering.dot(up);
    let throttle = flat.length().min(1.0);
    flat.normalize_or_zero() * walk.max_speed * throttle
}

#[cfg(test)]
mod tests;
