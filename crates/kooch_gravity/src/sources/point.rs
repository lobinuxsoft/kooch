//! [`PointGravity`] — a field pulling towards one point.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// A field pulling towards this entity — a planet, a moon, a black hole — moving and parenting with
/// it.
///
/// # Sizing one
///
/// A body stays grounded while gravity covers its centripetal acceleration; faster it orbits, and
/// escape takes `sqrt(2)` more:
///
/// ```text
/// stays on the ground   v <= sqrt(g · r)
/// leaves for good       v >= sqrt(2 · g · r)     with unlimited range
/// ```
///
/// `r` is the body's centre: planet radius plus its own.
///
/// ## Getting `g`
///
/// `strength` is quoted at `radius` and clamped inside it:
///
/// ```text
/// g = s · (R / max(r, R))²
/// ```
///
/// A `radius` beyond anything standing on the planet gives a flat `g = s`, which makes a small
/// world feel solid.
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
/// Inside `radius` the clamp flattens the field and `v = sqrt(s · r)`.
///
/// For a small body `R ≈ v²/s`: twice the speed needs four times the planet — at 9.81, 8 m/s wants
/// 7 m and 20 m/s wants 41 m.
///
/// More `strength` costs jump height, `(J/m)²/(2·g)`: on a 4 m planet, 9.81 → 25 buys 5.9 → 9.4 m/s
/// and costs 2.32 → 0.91 m of jump.
///
/// ## `range` lowers the escape speed
///
/// Past `range` there is nothing to climb against, so escaping costs only the pull out to the
/// cutoff:
///
/// ```text
/// r <  R:   v = sqrt( 2 · ( s·(R − r) + s·R²·(1/R − 1/range) ) )
/// r >= R:   v = sqrt( 2 · s·R² · (1/r − 1/range) )
/// ```
///
/// A `falloff` extends the field past `range`, so escaping costs a little more than the formulas
/// above: they assume the hard edge that a zero `falloff` is.
///
/// Beyond `range`, [`gravity_up`](crate::gravity_up) answers world up and the controls turn
/// world-relative — keep `range` past anything the player reaches.
///
/// Metres, like every source: scaling the entity moves the field without resizing it. Defaults to
/// about Earth's pull at a 50 m radius.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct PointGravity {
    /// Acceleration at [`radius`](Self::radius), in m/s² — authored as a number at a distance, not
    /// as `G·M`.
    pub strength: f32,
    /// The distance at which the field is exactly `strength`.
    pub radius: f32,
    /// Beyond this the source contributes nothing — summing every source in a galaxy has no
    /// gameplay behind it. Zero or less is unlimited.
    pub range: f32,
    /// Fall off with the square of distance. Off gives constant strength inside `range`:
    /// unphysical, and often what a walkable planet wants.
    pub inverse_square: bool,
    /// How far past `range` the field fades to nothing, in metres — the pull and, with a
    /// [`GravityPriority`](crate::GravityPriority), the claim over lower levels. Zero is a hard edge,
    /// which a priority turns into a jolt: the planet takes over all at once.
    pub falloff: f32,
}

impl Default for PointGravity {
    fn default() -> Self {
        Self {
            strength: 9.81,
            radius: 50.0,
            range: 500.0,
            inverse_square: true,
            falloff: 0.0,
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
        let reach = self.influence(distance);
        if reach <= 0.0 {
            return Vec3::ZERO;
        }

        let magnitude = match self.inverse_square {
            // Clamped at the reference radius, or the pull goes to infinity near the centre and
            // launches things out of the world.
            true => self.strength * (self.radius / distance.max(self.radius)).powi(2),
            false => self.strength,
        };
        direction * magnitude * reach
    }

    /// How strongly the field reaches `distance` metres out: 1 inside `range`, fading to 0 across
    /// `falloff`, 0 beyond. Unlimited when `range` is zero or less.
    pub fn influence(&self, distance: f32) -> f32 {
        if self.range <= 0.0 || distance <= self.range {
            return 1.0;
        }
        if self.falloff <= 0.0 {
            return 0.0;
        }
        (1.0 - (distance - self.range) / self.falloff).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests;
