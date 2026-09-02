//! [`PointGravity`] — a field pulling towards one point.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// A field pulling towards this entity — a planet, a moon, a black hole.
///
/// The direction is towards the entity's own position, so moving the
/// entity moves the field, and parenting it to something makes the field
/// follow.
///
/// # Sizing one
///
/// A body stays on the surface only while gravity covers the centripetal
/// acceleration its own speed demands. Past that it does not fall off —
/// it *orbits*, circling without touching. Leaving for good takes more
/// still, and the two thresholds are a factor of `sqrt(2)` apart on
/// every planet:
///
/// ```text
/// stays on the ground   v <= sqrt(g · r)
/// leaves for good       v >= sqrt(2 · g · r)     with unlimited range
/// ```
///
/// `r` is where the body's centre sits — the planet's radius plus the
/// body's own — and `g` is the pull there. In between the two the field
/// still holds: that is what an orbit is, and it is where the ISS lives.
///
/// ## Getting `g`
///
/// `strength` is quoted *at* `radius`, and clamped inside it so the pull
/// towards a centre stays finite:
///
/// ```text
/// g = s · (R / max(r, R))²
/// ```
///
/// So `radius` set at the surface gives `g = s·R²/r²`, and `radius` set
/// beyond anything that stands on the planet gives a flat `g = s` — a
/// field of constant strength near the surface, which is often what a
/// small world wants and what makes it feel solid.
///
/// ## Solving for the third number
///
/// With `radius` at the surface and a body of radius `b`, so `r = R + b`:
///
/// ```text
/// top speed   v = sqrt( s·R² / (R + b) )
/// strength    s = v² · (R + b) / R²
/// radius      R = ( v² + sqrt( v⁴ + 4·s·v²·b ) ) / 2s
/// ```
///
/// These assume the body is *outside* `radius`. Inside it the clamp
/// flattens the field and the answer is simply `v = sqrt(s · r)`.
///
/// For a body much smaller than its planet: `v ≈ sqrt(s·R)` and
/// `R ≈ v²/s`. Speed is squared and radius is not, so twice the top speed
/// needs four times the planet — at 9.81, 8 m/s wants a 7 m radius and
/// 20 m/s wants 41 m. Earth's 6371 km holds 7.9 km/s, which is why none
/// of this ever comes up on a flat level.
///
/// Raising `strength` instead is not free: jump height is `(J/m)²/(2·g)`,
/// off the same `g`. On a 4 m planet, 9.81 -> 25 buys 5.9 -> 9.4 m/s of
/// grip and costs 2.32 -> 0.91 m of jump. Growing the planet keeps both.
///
/// ## `range` lowers the escape speed
///
/// Past `range` there is no field left to climb against, so leaving costs
/// less than the unlimited `sqrt(2·g·r)`. It costs the pull integrated
/// out to the cutoff:
///
/// ```text
/// r <  R:   v = sqrt( 2 · ( s·(R − r) + s·R²·(1/R − 1/range) ) )
/// r >= R:   v = sqrt( 2 · s·R² · (1/r − 1/range) )
/// ```
///
/// Beyond `range` [`gravity_up`](crate::gravity_up) answers world up, so
/// the controls turn world-relative between one step and the next. That
/// reads as the gravity breaking, and it is why `range` wants to sit past
/// anything the player can reach.
///
/// # The entity's scale does not resize this
///
/// `radius` and `range` are metres, as they are on every other source.
/// Scaling the entity moves the field without resizing it.
///
/// # Default
///
/// Roughly Earth's surface gravity at a 50 m radius, which is a planet you
/// can walk around in a test scene rather than one you would need a
/// telescope to see.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct PointGravity {
    /// Acceleration at [`radius`](Self::radius), in metres per second
    /// squared.
    ///
    /// Given at a distance rather than as a mass so it can be authored
    /// directly: "9.81 at the surface" is a number someone can reason
    /// about, and `G·M` is not.
    pub strength: f32,
    /// The distance at which the field is exactly `strength`.
    pub radius: f32,
    /// Beyond this, the source contributes nothing.
    ///
    /// A cutoff rather than an infinite field: real gravity never reaches
    /// zero, and summing every source in a galaxy for every body is a cost
    /// with no gameplay behind it. Zero or less means unlimited.
    pub range: f32,
    /// Fall off with the square of distance, as gravity does.
    ///
    /// Off gives a field of constant strength inside `range`, which is not
    /// physical and is often what a game wants: a small planet you can
    /// walk on without the pull changing under your feet.
    pub inverse_square: bool,
}

impl Default for PointGravity {
    fn default() -> Self {
        Self {
            strength: 9.81,
            radius: 50.0,
            range: 500.0,
            inverse_square: true,
        }
    }
}

impl Component for PointGravity {}

impl PointGravity {
    /// The acceleration this source applies at `point`, given its own
    /// world position.
    pub fn acceleration_at(&self, source: Vec3, point: Vec3) -> Vec3 {
        let offset = source - point;
        let distance = offset.length();
        // At the centre there is no direction to pull in, and dividing by
        // it would produce NaN that outlives the frame.
        let Some(direction) = offset.try_normalize() else {
            return Vec3::ZERO;
        };
        if self.range > 0.0 && distance > self.range {
            return Vec3::ZERO;
        }

        let magnitude = match self.inverse_square {
            // Clamped at the reference radius rather than growing without
            // bound: inside a planet the pull should not go to infinity as
            // a body approaches the centre, which is both unphysical and a
            // reliable way to launch something out of the world.
            true => self.strength * (self.radius / distance.max(self.radius)).powi(2),
            false => self.strength,
        };
        direction * magnitude
    }
}

#[cfg(test)]
mod tests;
