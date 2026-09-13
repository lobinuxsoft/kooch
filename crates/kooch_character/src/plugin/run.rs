//! Running along a wall: the clock, and what it holds off.

use std::collections::HashMap;

use glam::Vec3;

use kooch_ecs::entity::Entity;

use crate::wall_run::WallRun;

/// Where a character stands with the wall it is on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Run {
    /// Running, and this far into the clock.
    Going(f32),
    /// This wall has been answered for and the answer was no — too slow
    /// on arrival, or the clock is out. Leaving the wall or landing is
    /// what clears it.
    Refused,
}

/// Every character's answer for the wall it is currently on.
#[derive(Default)]
pub struct Runs {
    runs: HashMap<Entity, Run>,
}

impl Runs {
    /// How far into a run this character is, or `None` for not running.
    pub fn of(&self, entity: Entity) -> Option<f32> {
        match self.runs.get(&entity) {
            Some(Run::Going(elapsed)) => Some(*elapsed),
            _ => None,
        }
    }

    /// What it is doing about this wall, if it has decided yet.
    pub fn state(&self, entity: Entity) -> Option<Run> {
        self.runs.get(&entity).copied()
    }

    pub fn set(&mut self, entity: Entity, run: Run) {
        self.runs.insert(entity, run);
    }

    /// Off the wall, or back on the ground: the next wall is a fresh
    /// question.
    pub fn landed(&mut self, entity: Entity) {
        self.runs.remove(&entity);
    }

    pub fn forget(&mut self, entity: Entity) {
        self.runs.remove(&entity);
    }
}

/// The part of a velocity that runs along a wall, given which way is up.
pub fn along(velocity: Vec3, normal: Vec3, up: Vec3) -> Vec3 {
    let flat = velocity - up * velocity.dot(up);
    flat - normal * flat.dot(normal)
}

/// Whether a run starts or carries on. Entry speed is asked only on the first step, or a slow
/// arrival could steer itself up to a run; `Refused` sticks until the character leaves or lands.
pub fn carry(state: Option<Run>, speed: f32, run: &WallRun, dt: f32) -> Run {
    let spent = match state {
        Some(Run::Refused) => return Run::Refused,
        Some(Run::Going(spent)) => spent,
        None if speed >= run.entry.max(0.0) => 0.0,
        // Arrived too slowly. This wall is spent.
        None => return Run::Refused,
    };
    let spent = spent + dt;
    match spent <= run.duration.max(0.0) {
        true => Run::Going(spent),
        false => Run::Refused,
    }
}

/// How the body sits while running: tilted from `up` towards the wall, away from `normal` — upright
/// reads as hovering beside it.
pub fn banked(up: Vec3, normal: Vec3, bank: f32) -> Vec3 {
    let Some(up) = up.try_normalize() else {
        return up;
    };
    let Some(normal) = normal.try_normalize() else {
        return up;
    };
    let bank = bank.clamp(0.0, 1.0);
    (up * (1.0 - bank) + normal * bank).normalize_or(up)
}

#[cfg(test)]
mod tests;
