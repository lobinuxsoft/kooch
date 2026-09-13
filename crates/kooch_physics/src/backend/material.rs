//! How a surface behaves on contact — authorable since #623; before, every collider took rapier's
//! defaults, chosen by nobody.

/// How two colliders' coefficients combine. Rapier takes the higher discriminant, so the pushier
/// rule wins: [`Average`](Self::Average) against [`Max`](Self::Max) gets `Max`, for every pair that
/// surface touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CombineRule {
    /// The mean. Rapier's default and the ordinary choice.
    #[default]
    Average,
    /// The smaller value — the slipperier or softer surface wins.
    Min,
    /// The product. Both surfaces have to be high for the result to be.
    Multiply,
    /// The larger value — the stickier or bouncier surface wins.
    Max,
    /// The sum, clamped.
    ClampedSum,
}

/// A collider's surface on contact, separate from [`CollisionShape`](super::CollisionShape): the
/// same box is ice or rubber. Defaults are rapier's — half friction, no bounce, averaged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceMaterial {
    /// Resistance to sliding. 0 is frictionless; 1 is roughly rubber on
    /// dry tarmac. Values above 1 are legal and useful for gameplay.
    pub friction: f32,
    pub friction_rule: CombineRule,
    /// Bounce. 0 keeps all the impact; 1 returns it, so a ball comes back
    /// to about the height it fell from.
    pub restitution: f32,
    pub restitution_rule: CombineRule,
}

impl Default for SurfaceMaterial {
    fn default() -> Self {
        Self {
            friction: 0.5,
            friction_rule: CombineRule::Average,
            restitution: 0.0,
            restitution_rule: CombineRule::Average,
        }
    }
}

impl SurfaceMaterial {
    /// Clamps coefficients into the solver's range: negative values push bodies together, and an
    /// Inspector edit passes through them.
    pub fn sanitised(self) -> Self {
        Self {
            friction: self.friction.max(0.0),
            restitution: self.restitution.max(0.0),
            ..self
        }
    }
}

/// Motion lost with no contact — air, not ice. Zero on both by default, as rapier; #618's sluggish
/// rotation was not damping.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Damping {
    pub linear: f32,
    pub angular: f32,
}

impl Damping {
    /// Clamps to non-negative. A negative damping is an amplifier, and a
    /// body that gains energy every step leaves the number line.
    pub fn sanitised(self) -> Self {
        Self {
            linear: self.linear.max(0.0),
            angular: self.angular.max(0.0),
        }
    }
}

#[cfg(test)]
mod tests;
