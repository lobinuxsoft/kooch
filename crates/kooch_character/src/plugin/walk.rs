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
    seen: HashMap<Entity, Vec3>,
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

    /// Puts a character's goal where its velocity already is.
    ///
    /// What a body in the air does every step, so it lands chasing
    /// reality: a goal left over from before the jump would be spent on
    /// the landing frame as a shove in whatever direction it had.
    pub fn hold(&mut self, entity: Entity, velocity: Vec3) {
        self.goals.insert(entity, velocity);
    }

    /// The acceleration a character actually got, from the velocity it
    /// actually has.
    ///
    /// Not the force applied: a body shoving a wall is given the full
    /// `max_force` and goes nowhere, and a lean drawn from that tips the
    /// character over at 29 degrees and leaves it there. A lean is a
    /// response to changing speed, so it has to be measured from speed.
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

/// Steering in the air, where there is nothing to push against.
///
/// The goal-velocity chase is wrong here and reads as it: it brakes
/// towards zero, so letting go of the stick mid-jump stops the
/// character dead in the air. Momentum is what a jump *is*.
///
/// So the air only ever adds, in the direction being asked for, and
/// never past the speed the body arrived with — steerable, and unable
/// to turn into thrust.
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

/// A push with the part heading into a wall taken out.
///
/// `normal` points from the surface back at the character, so a push
/// into it is the negative part. Only that part goes: steering *along*
/// a wall is how a character rounds a corner.
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
