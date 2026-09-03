//! Spending a jump, once the sense pass has said what is available.
//!
//! Reads [`Grounded`] and [`Touching`]; casts nothing. The forgiveness
//! windows are the whole reason this is not two lines: a jump is
//! allowed slightly after leaving the ground and slightly before
//! arriving, and both timers live here.

use std::collections::HashMap;

use glam::Vec3;

use kooch_ecs::entity::Entity;

use crate::jump::{Jump, WallJump};

/// One character's jump timers and how many it has left.
#[derive(Clone, Copy, Default)]
pub struct Tally {
    /// Seconds since it last had ground under it.
    pub ungrounded: f32,
    /// Seconds since the button went down, or `None` for not pressed.
    pub asked: Option<f32>,
    /// Air jumps spent since the last time it stood on something.
    pub spent: u32,
    /// Seconds since it pushed off a wall, or `None` for never.
    ///
    /// A wall slide holds the character on, and the frame after a wall
    /// jump the wall is still right there — so the hold would cancel the
    /// jump it just made. This is how long it is left alone to get
    /// clear.
    pub since_wall: Option<f32>,
}

/// Every character's jump state, carried between steps.
#[derive(Default)]
pub struct Tallies {
    tallies: HashMap<Entity, Tally>,
}

impl Tallies {
    pub fn of(&self, entity: Entity) -> Tally {
        self.tallies.get(&entity).copied().unwrap_or_default()
    }

    pub fn set(&mut self, entity: Entity, tally: Tally) {
        self.tallies.insert(entity, tally);
    }

    pub fn forget(&mut self, entity: Entity) {
        self.tallies.remove(&entity);
    }
}

/// How long after pushing off a wall the slide leaves the character
/// alone, in seconds.
///
/// Long enough to clear the wall's own reach, short enough that a
/// deliberate return to it still catches.
pub const CLEARING: f32 = 0.25;

/// What a jump turns into, if anything.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Leap {
    /// Straight up the local up.
    Ground(Vec3),
    /// Away from a wall and up it.
    Wall(Vec3),
}

/// Advances the timers and decides whether a jump happens this step.
///
/// `up` is the local up, `wall` the normal of anything in reach.
pub fn spend(
    tally: &mut Tally,
    jump: &Jump,
    wall: Option<(&WallJump, Vec3)>,
    standing: bool,
    up: Vec3,
    dt: f32,
) -> Option<Leap> {
    if let Some(since) = tally.since_wall.as_mut() {
        *since += dt;
    }
    tally.ungrounded = match standing {
        true => 0.0,
        false => tally.ungrounded + dt,
    };
    if standing {
        tally.spent = 0;
    }
    if jump.wanted {
        tally.asked = Some(0.0);
    } else if let Some(waited) = tally.asked.as_mut() {
        *waited += dt;
    }

    // Nothing asked, or asked so long ago that honouring it would be a
    // jump the player has stopped expecting.
    let waited = tally.asked?;
    if waited > jump.buffer.max(0.0) {
        tally.asked = None;
        return None;
    }

    // Off a wall first. It is the move the player meant if they are
    // against one, and it is the only one available once the air jumps
    // are gone.
    if !standing
        && let Some((off, normal)) = wall
        && let Some(away) = (normal - up * normal.dot(up)).try_normalize()
    {
        tally.asked = None;
        tally.since_wall = Some(0.0);
        if off.refills {
            tally.spent = 0;
        }
        return Some(Leap::Wall(away * off.push + up * off.climb));
    }

    // On the ground, or close enough after leaving it that the player
    // still believes they are.
    if standing || tally.ungrounded <= jump.coyote.max(0.0) {
        tally.asked = None;
        // Coyote time is a grace, not a free jump: taking it has to cost
        // the ground jump, or a character walking off a ledge gets one
        // more jump than one that stood still.
        tally.ungrounded = f32::MAX;
        return Some(Leap::Ground(up * jump.speed));
    }

    if tally.spent < jump.air_jumps {
        tally.asked = None;
        tally.spent += 1;
        return Some(Leap::Ground(up * jump.speed));
    }
    None
}

#[cfg(test)]
mod tests;
