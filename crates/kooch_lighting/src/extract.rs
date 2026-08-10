//! The ECS walk: light components + their world transforms → the flat
//! record the shader loops over.
//!
//! Pure and `Resources`-only, so the whole extraction is testable with
//! no GPU in the room. The buffer upload is [`crate::GpuLights`]'s job.

use kooch_core::resource::Resources;
use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::query::Query;
use kooch_ecs::spot_light::SpotLight;

use crate::gpu_light::GpuLight;

/// Light count past which the shader's linear loop stops being an
/// honest implementation and starts being a performance bug. Warned
/// about, never enforced: clipping a scene's lights silently is worse
/// than rendering it slowly, and the caller can see the log.
///
/// The fix is clustering, deliberately out of scope for #441 — Bevy
/// moved theirs to the GPU in 0.18/0.19 and measured ~20× on their
/// `many_lights` benchmark. A universe has stars.
const LINEAR_LOOP_BUDGET: usize = 256;

/// The lights as the shader reads them, plus the entity each one came
/// from, in the same order.
///
/// Two parallel vectors rather than an entity field on [`GpuLight`]:
/// that record is a 64 B POD that crosses to the GPU, and an identity
/// the shader will never read has no business riding in it.
pub struct ExtractedLights {
    pub lights: Vec<GpuLight>,
    pub entities: Vec<Entity>,
}

impl ExtractedLights {
    /// Which slot of the light buffer `entity` landed in, if it is a
    /// light this frame is rendering.
    ///
    /// A linear scan, deliberately: the walk order is the buffer order
    /// and a map would be a second structure to keep in step with it.
    /// The array is at most a few hundred entries and only the editor
    /// asks (#743).
    pub fn slot_of(&self, entity: Entity) -> Option<u32> {
        self.entities
            .iter()
            .position(|e| *e == entity)
            .map(|i| i as u32)
    }
}

/// Walks the world for every active light and packs it for the GPU.
///
/// A light with no `GlobalTransform` is skipped rather than defaulted:
/// a directional light without a transform has no direction, and
/// placing it at the origin pointing down would be an invention.
///
/// 🔴 The order this walks in **is** the layout of the light buffer, and
/// [`ExtractedLights::slot_of`] resolves against it. One walk produces
/// both, because two that disagreed would isolate the wrong light in the
/// debug view and nothing would report it.
pub fn extract_lights(resources: &Resources) -> ExtractedLights {
    let mut lights = Vec::new();
    let mut entities = Vec::new();

    Query::<(&DirectionalLight, &GlobalTransform)>::new(resources).for_each_entity(
        |entity, (light, transform)| {
            if light.active {
                lights.push(GpuLight::directional(light, transform.matrix));
                entities.push(entity);
            }
        },
    );
    Query::<(&PointLight, &GlobalTransform)>::new(resources).for_each_entity(
        |entity, (light, transform)| {
            if light.active {
                lights.push(GpuLight::point(light, transform.matrix));
                entities.push(entity);
            }
        },
    );
    // The spot walk also hands out shadow slots, in this order, because
    // `shadow_casting_spots` reads the same order back and the two have
    // to be the same numbering. A spot past the budget still lights the
    // scene with `NO_SHADOW_SLOT` — losing the light would be a worse
    // failure than losing its shadow.
    let mut next_slot = 0u32;
    Query::<(&SpotLight, &GlobalTransform)>::new(resources).for_each_entity(
        |entity, (light, transform)| {
            if light.active {
                let mut gpu = GpuLight::spot(light, transform.matrix);
                if light.cast_shadows && (next_slot as usize) < crate::MAX_SPOT_SHADOWS {
                    gpu.shadow_slot = next_slot;
                    next_slot += 1;
                }
                lights.push(gpu);
                entities.push(entity);
            }
        },
    );

    ExtractedLights { lights, entities }
}

/// The direction of the one directional light that casts shadows, if
/// the scene has one.
///
/// Points **where the light shines** — the entity's -Z, the same vector
/// the shading model reads — so a caller can hand it straight to
/// `build_cascades`.
///
/// # Why the first and not all of them
///
/// The atlas holds four cascades of one light. A second sun would need
/// a second atlas, and there is no bind group left to put it in. Taking
/// the first in walk order is a limitation stated rather than a choice:
/// a scene with two shadow-casting suns gets shadows from one of them,
/// which is visibly wrong and therefore reportable, as opposed to
/// getting none, which reads as the feature being broken.
pub fn shadow_casting_sun(resources: &Resources) -> Option<glam::Vec3> {
    let mut found = None;
    Query::<(&DirectionalLight, &GlobalTransform)>::new(resources).for_each(
        |(light, transform)| {
            if found.is_none() && light.active && light.cast_shadows {
                found = Some(crate::gpu_light::forward(transform.matrix));
            }
        },
    );
    found
}

/// Everything the shadow pass needs about one spot light that casts.
///
/// The angle is the OUTER one: the cone's edge is where the light stops,
/// so a frustum fitted to anything narrower clips the lit region and
/// leaves a hard square inside a round pool of light.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SpotShadowSource {
    pub entity: Entity,
    pub position: glam::Vec3,
    /// Where the light shines — the entity's -Z, same as `GpuLight`.
    pub direction: glam::Vec3,
    pub outer_angle: f32,
    pub range: f32,
}

/// The spot lights that cast, in the order their shadows are numbered.
///
/// 🔴 The order is `extract_lights`'s, and the slot each one gets here is
/// the `shadow_slot` written into its `GpuLight`. Two walks that disagree
/// would light a spot with another spot's shadow map — geometry from
/// somewhere else in the room, which reads as a broken shadow pass and
/// not as a mismatched index. That is why this calls the same walk
/// rather than repeating its filter.
pub fn shadow_casting_spots(resources: &Resources, limit: usize) -> Vec<SpotShadowSource> {
    let mut out = extract_spot_sources(resources);
    out.truncate(limit);
    out
}

fn extract_spot_sources(resources: &Resources) -> Vec<SpotShadowSource> {
    let mut out = Vec::new();
    Query::<(&SpotLight, &GlobalTransform)>::new(resources).for_each_entity(
        |entity, (light, transform)| {
            if light.active && light.cast_shadows {
                out.push(SpotShadowSource {
                    entity,
                    position: transform.matrix.w_axis.truncate(),
                    direction: crate::gpu_light::forward(transform.matrix),
                    outer_angle: light.outer_angle,
                    range: light.range,
                });
            }
        },
    );
    out
}

/// What shadow one light actually casts, in words, for the editor to
/// show next to the single-light debug view (#743).
///
/// # Why this is text and not something the view draws
///
/// A punctual light has no shadow map: the cascades are fit to the view
/// frustum for a light with no position, and a point light would need a
/// cube map instead (#734 is the other half). Contact shadows are the
/// only occlusion it can have, and they are opt-in and off by default
/// there — fifty lamps should not each cost a screen-space march.
///
/// So "this light casts nothing" is the common, correct answer for a
/// point light, and it renders **identically** to a shadow that failed.
/// The view cannot distinguish them because there is nothing to draw.
/// A sentence can.
///
/// 🔴 An **inactive** light reports that it is inactive rather than
/// reporting nothing.
///
/// A light with `active == false` never reaches the buffer, so it has no
/// slot and the view renders magenta — the same magenta as selecting a
/// crate. Those are different facts with different fixes ("tick the box"
/// against "select a light"), and a view whose whole purpose is to stop
/// two causes from looking alike must not introduce a third pair.
///
/// `None` only when the entity is not a light at all.
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
    // `cast_shadows` is deliberately not consulted below: there is no
    // shadow map for a punctual light to cast into, so the field
    // promises something the engine does not do yet. Reporting it would
    // be worse than saying nothing.
    if let Some(light) = Query::<&PointLight>::new(resources).get(entity) {
        if !light.active {
            return Some(INACTIVE_NOTE);
        }
        return Some(if light.contact_shadows {
            "Point: contact shadows only — no shadow map exists for punctual lights yet"
        } else {
            "Point: casts no shadow — no shadow map, and contact shadows are off"
        });
    }
    if let Some(light) = Query::<&SpotLight>::new(resources).get(entity) {
        if !light.active {
            return Some(INACTIVE_NOTE);
        }
        return Some(if light.contact_shadows {
            "Spot: contact shadows only — no shadow map exists for punctual lights yet"
        } else {
            "Spot: casts no shadow — no shadow map, and contact shadows are off"
        });
    }
    None
}

/// What a light that is switched off says. It names the checkbox,
/// because that is the whole of the fix.
const INACTIVE_NOTE: &str = "This light is inactive — tick `active` in the Inspector to see it";

/// `Some(count)` when `lights` is past the budget the shader's linear
/// loop can carry honestly, `None` otherwise.
///
/// Separate from the walk because the walk runs once **per view per
/// frame** — warning inside it would emit sixty lines a second per open
/// panel, and the advice would be the thing making the log unreadable.
/// [`crate::GpuLights::update`] warns on the transition instead.
pub(crate) fn over_linear_budget(lights: usize) -> Option<usize> {
    (lights > LINEAR_LOOP_BUDGET).then_some(LINEAR_LOOP_BUDGET)
}

// Tests live in `tests/extraction.rs`: exercising the walk needs a
// real `ComponentRegistry` + `ArchetypeRegistry`, which is an
// integration-shaped fixture, not a unit-shaped one.
