//! The walk's records and ranking; the walk itself is [`crate::LightFrame`], so archetypes are read
//! once per frame. `Resources`-only, testable without a GPU; the upload is [`crate::GpuLights`].

use kooch_core::resource::Resources;
use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::entity::Entity;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::query::Query;
use kooch_ecs::spot_light::SpotLight;

use crate::gpu_light::GpuLight;

/// Lights past which the linear loop is a performance bug — warned, never enforced. Since #780 it
/// only binds when the grid is off (no matrices, `KOOCH_CLUSTERING=off`).
const LINEAR_LOOP_BUDGET: usize = 256;

/// Lights as the shader reads them, with their entities in parallel: an 80 B GPU record has no room
/// for an identity it never reads.
#[derive(Clone)]
pub struct ExtractedLights {
    pub lights: Vec<GpuLight>,
    pub entities: Vec<Entity>,
    /// Directional lights at the buffer's start. 🔴 A prefix, not a subset: the shader walks
    /// `0..directional_count` linearly and takes the rest from the grid.
    pub directional_count: u32,
}

impl ExtractedLights {
    /// The buffer slot `entity` landed in — a linear scan over a few hundred entries, since walk
    /// order is buffer order; only the editor asks (#743).
    pub fn slot_of(&self, entity: Entity) -> Option<u32> {
        self.entities
            .iter()
            .position(|e| *e == entity)
            .map(|i| i as u32)
    }
}

/// One casting spot, with its OUTER angle — a frustum fitted narrower clips the pool into a square.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SpotShadowSource {
    pub entity: Entity,
    /// The spot's buffer slot. 🔴 Not `entity`: a slot means nothing next frame, the entity
    /// survives.
    pub buffer_slot: u32,
    pub position: glam::Vec3,
    /// Where the light shines — the entity's -Z, same as `GpuLight`.
    pub direction: glam::Vec3,
    /// 🔴 Outer half-angle in radians, converted from `SpotLight`'s degrees here — 45 read as
    /// radians is a hemisphere-wide map.
    pub outer_angle: f32,
    pub range: f32,
    /// Layers that cast into this light's shadow (#1220): the mask its shadow view culls with.
    pub shadow_layers: u32,
}

/// Writes each casting point light's cube slot from `casting`'s ranking — walk order is not slot
/// order, so assigning during the walk lights lamps with others' cubes.
/// 🔴 Idempotent: every view reruns it, so every point slot is cleared first. Spots are not touched.
pub fn assign_point_slots(lights: &mut ExtractedLights, casting: &[Entity]) {
    for light in &mut lights.lights {
        if light.kind == crate::gpu_light::LIGHT_KIND_POINT {
            light.shadow_slot = crate::gpu_light::NO_SHADOW_SLOT;
        }
    }
    for (slot, entity) in casting.iter().enumerate() {
        if let Some(index) = lights.slot_of(*entity) {
            lights.lights[index as usize].shadow_slot = slot as u32;
        }
    }
}

/// One point light's cube shadow (#778); no direction, which is why it costs six faces.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PointShadowSource {
    pub entity: Entity,
    /// The lamp's buffer slot. 🔴 A position this frame, not identity; hysteresis matches on
    /// `entity`.
    pub buffer_slot: u32,
    pub position: glam::Vec3,
    pub range: f32,
    /// The light's own brightness, carried because the ranking needs it
    /// and the component is out of reach by then.
    pub intensity: f32,
    /// How much a cube on this light would show this frame ([`point_shadow_importance`]) — derived
    /// from the camera, never stored.
    pub importance: f32,
    /// Layers that cast into this light's shadow (#1220): the mask its six faces cull with.
    pub shadow_layers: u32,
}

/// What a cube on this light would show: `(range / distance)²` clamped at 1, times `intensity`. 🔴
/// Replaces nearest-first, which reshuffled every step; `select_point_casters` in `kooch_render`
/// adds hysteresis.
pub fn point_shadow_importance(
    position: glam::Vec3,
    range: f32,
    intensity: f32,
    camera_position: glam::Vec3,
) -> f32 {
    let distance = position.distance(camera_position).max(1e-4);
    let angular = (range / distance).min(1.0);
    intensity.max(0.0) * angular * angular
}

/// The shadow one light casts, in words (#743): nothing and failure render the same, a sentence
/// differs. An inactive light says so rather than showing a non-light's magenta. `None` if not a
/// light.
pub fn shadow_note(resources: &Resources, entity: Entity) -> Option<&'static str> {
    if let Some(light) = Query::<&DirectionalLight>::new(resources).get(entity) {
        if !light.active {
            return Some(INACTIVE_NOTE);
        }
        return Some(match (light.cast_shadows, light.contact_shadows) {
            (true, true) => "Directional: cascades + contact shadows",
            (true, false) => "Directional: cascades, contact shadows off",
            (false, true) => "Directional: contact shadows only, cascades off",
            (false, false) => "Directional: casts nothing — both shadow options are off",
        });
    }
    // Since #778 a point light's `cast_shadows` is real. ⚠️ Whether it won a cube is decided per
    // frame by importance, which the Inspector cannot see.
    if let Some(light) = Query::<&PointLight>::new(resources).get(entity) {
        if !light.active {
            return Some(INACTIVE_NOTE);
        }
        return Some(match (light.cast_shadows, light.contact_shadows) {
            (true, true) => "Point: cube map + contact shadows",
            (true, false) => "Point: cube map, contact shadows off",
            (false, true) => "Point: contact shadows only — its cube map is off",
            (false, false) => "Point: casts nothing — both shadow options are off",
        });
    }
    if let Some(light) = Query::<&SpotLight>::new(resources).get(entity) {
        if !light.active {
            return Some(INACTIVE_NOTE);
        }
        // A spot has had a shadow map since #777, so unlike a point
        // light its `cast_shadows` is a promise the engine keeps.
        return Some(match (light.cast_shadows, light.contact_shadows) {
            (true, true) => "Spot: shadow map + contact shadows",
            (true, false) => "Spot: shadow map, contact shadows off",
            (false, true) => "Spot: contact shadows only — its shadow map is off",
            (false, false) => "Spot: casts nothing — both shadow options are off",
        });
    }
    None
}

/// What a light that is switched off says. It names the checkbox,
/// because that is the whole of the fix.
const INACTIVE_NOTE: &str = "This light is inactive — tick `active` in the Inspector to see it";

/// `Some(count)` past the linear-loop budget. Separate from the walk, which runs per view per
/// frame; [`crate::GpuLights::update`] warns on the transition.
pub(crate) fn over_linear_budget(lights: usize) -> Option<usize> {
    (lights > LINEAR_LOOP_BUDGET).then_some(LINEAR_LOOP_BUDGET)
}

// Tests live in `tests/extraction.rs`: exercising the walk needs a
// real `ComponentRegistry` + `ArchetypeRegistry`, which is an
// integration-shaped fixture, not a unit-shaped one.
