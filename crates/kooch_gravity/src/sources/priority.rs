//! [`GravityPriority`] — the zone that overrules the planet.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Which sources a field overrules, on the entity carrying that field.
///
/// Fields add, and that is the right answer almost always: two planets pull
/// along the vector sum, and a body between them transitions smoothly
/// because the arithmetic already says so. What summing cannot express is a
/// zone that *replaces* — "inside this room down is -X, ignore the planet".
/// Summed, the room fights the planet and the result is a diagonal nobody
/// authored.
///
/// A source with a higher `level` suppresses every lower one in proportion
/// to how strongly it reaches a point. Sources at the same level sum, as
/// they always did.
///
/// # Absent means zero
///
/// A source without this component sits at level 0, so adding the component
/// to one entity in a scene changes nothing else. Levels may be negative,
/// which is how a background field is put *under* the default rather than
/// every other source being lifted above it.
///
/// # The suppression is gradual, so give the zone a soft edge
///
/// At a point where the overriding source reaches full strength the lower
/// levels are gone; across its fade they come back in proportion. This is
/// what keeps a body from snapping direction as it crosses a boundary, and
/// it means the shape of the transition is the shape of that source's own
/// falloff.
///
/// [`AreaGravity`](super::AreaGravity), [`BoxGravity`](super::BoxGravity)
/// and [`PlaneGravity`](super::PlaneGravity) have a `falloff` band for
/// exactly this. [`PointGravity`](super::PointGravity) claims everything
/// inside its `range` and nothing outside, so overriding with one is a hard
/// edge; [`GlobalGravity`](super::GlobalGravity) reaches everywhere at full
/// strength, so raising *it* switches the rest of the scene off entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Reflect)]
#[reflect(category = "Physics")]
pub struct GravityPriority {
    /// Higher overrules lower. Equal levels sum.
    pub level: i32,
}

impl Component for GravityPriority {}
