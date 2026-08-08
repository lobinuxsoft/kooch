//! How a surface behaves on contact: friction and bounce.
//!
//! Until #623 none of this was authorable. Every collider the engine built
//! took rapier's defaults — 0.5 friction, no bounce — chosen by nobody, and
//! every body took zero damping. A scene could behave in a way no one had
//! asked for and no one could change.

/// How two colliders' coefficients combine into the one the solver uses.
///
/// # The rule is not negotiated
///
/// Rapier resolves a pair with `rule1.max(rule2)` over the discriminants,
/// so the *higher* variant wins outright: a collider asking for
/// [`Average`](Self::Average) against one asking for [`Max`](Self::Max)
/// gets `Max`, because `Max` is 3 and `Average` is 0.
///
/// That is worth knowing before authoring anything. A rule is not a
/// property of a surface so much as a claim about how it wants to be
/// combined, and the pushier claim wins. Leaving everything on `Average`
/// and setting one special surface to `Multiply` therefore affects every
/// pair that surface touches — which is usually the intent, but it is not
/// what "my collider's setting" sounds like.
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

/// What a collider's surface does on contact.
///
/// Separate from [`CollisionShape`](super::CollisionShape) because
/// geometry and surface are independent: the same box is ice or rubber
/// depending on this, and a shape has no opinion about either.
///
/// # Default
///
/// Rapier's own — half friction, no bounce, averaged. Those defaults are
/// the sane ones and #623 did not change them; the point was that they
/// were invisible and unchangeable, so a scene behaved in a way nobody had
/// chosen.
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
    /// Clamps the coefficients into the range the solver can use.
    ///
    /// A negative friction or restitution is not a slipperier surface, it
    /// is a solver that pushes bodies together on separation. A field
    /// mid-edit in the Inspector passes through negative on the way to a
    /// value the author means, so this is the same treatment collider
    /// dimensions get.
    pub fn sanitised(self) -> Self {
        Self {
            friction: self.friction.max(0.0),
            restitution: self.restitution.max(0.0),
            ..self
        }
    }
}

/// How quickly a body loses motion to nothing in particular.
///
/// Not friction: damping applies with no contact at all, which is what
/// makes it the right tool for "this should feel like it is moving through
/// air" and the wrong one for "this should slide less on ice".
///
/// # Default
///
/// Zero on both, rapier's own — a body in a vacuum keeps its motion. This
/// matters for #618's diagnosis: "the body rotates sluggishly" could not
/// have been damping, because nothing was damping it.
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
