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
/// 🔴 Since #780 the linear loop is the **fallback**, not the path: the
/// froxel grid walks a cell's lights instead of the scene's, and this
/// budget only binds when the grid is off — no camera matrices, or
/// `KOOCH_CLUSTERING=off`. Kept, and kept at the same number, because
/// that fallback is still what a headless path runs.
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
    /// How many directional lights the buffer opens with.
    ///
    /// 🔴 They are a **prefix**, not a subset, and the shading loop
    /// depends on it: clustering (#780) walks the punctual lights out of
    /// the froxel this fragment is in, and a directional light is in
    /// every froxel — so the grid says nothing about it. What the shader
    /// does instead is walk `0..directional_count` linearly, which is
    /// only correct while they come first.
    pub directional_count: u32,
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
    // Everything pushed above is directional, and everything below is
    // not. `directional_count` is that boundary, and the shading loop
    // reads it as one — see the field's documentation.
    let directional_count = lights.len() as u32;

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

    ExtractedLights {
        lights,
        entities,
        directional_count,
    }
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
    /// 🔴 The outer HALF-angle in **radians**, already converted.
    ///
    /// `SpotLight::outer_angle` is in degrees, like Unreal's, and it is
    /// a half-angle. Converting here rather than at the shadow frustum
    /// keeps the one place that reads the component next to the
    /// component: a 45 taken for radians is a 2578° cone, which clamps
    /// to the widest frustum allowed and produces a shadow map covering
    /// a hemisphere for a light that lights a doorway.
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

/// Writes each casting point light's cube slot into its `GpuLight`.
///
/// 🔴 Separate from `extract_lights`, and it has to be. A spot's slot is
/// handed out during the walk because `shadow_casting_spots` returns the
/// same walk order truncated — the two agree by construction. Point
/// lights are sorted by distance to the camera, so the walk order and
/// the slot order are **different orders**, and assigning during the
/// walk would light every lamp with another lamp's cube: geometry from
/// elsewhere in the room, which reads as a broken shadow pass rather
/// than as a mismatched index.
///
/// So the ranked list is the single source of truth and this looks each
/// entity back up in it, rather than the sort being repeated anywhere.
pub fn assign_point_slots(lights: &mut ExtractedLights, casting: &[Entity]) {
    for (slot, entity) in casting.iter().enumerate() {
        if let Some(index) = lights.slot_of(*entity) {
            lights.lights[index as usize].shadow_slot = slot as u32;
        }
    }
}

/// One point light's cube shadow, as the pass needs it (#778).
///
/// No direction: a cube map looks everywhere, which is the entire reason
/// it costs six faces where a spot costs one.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PointShadowSource {
    pub entity: Entity,
    pub position: glam::Vec3,
    pub range: f32,
    /// The light's own brightness, carried because the ranking needs it
    /// and the component is out of reach by then.
    pub intensity: f32,
    /// How much a cube spent on this light would show, this frame. See
    /// [`point_shadow_importance`].
    ///
    /// Derived from the camera, so it belongs to the frame rather than
    /// to the light: recomputed every time this list is built and never
    /// stored on the component.
    pub importance: f32,
}

/// How much a cube map spent on this light would show on screen.
///
/// Two factors, and distance is only half of one of them:
///
/// **How much of the screen the light can darken.** A light is a sphere
/// of `range`; the fraction of the view it can cover goes with the
/// square of its angular radius, `range / distance`. Clamped at 1: once
/// the camera is inside the sphere the light is all around the viewer
/// and getting closer to its centre does not make its shadow bigger.
///
/// **How much light there is to occlude.** A shadow is the absence of a
/// light's contribution, so a dim lamp casts a shadow nobody can see
/// standing next to a bright one. `intensity` alone and not
/// `intensity * luma(color)`: a saturated red lamp at high intensity is
/// still a bright lamp, and weighting by luma would quietly rank it
/// under a dim white one.
///
/// 🔴 This replaces sorting by distance. Nearest-first put the cubes on
/// whichever four lamps the camera happened to be closest to, and with
/// lights on a grid **the order changes every time the viewer walks a
/// metre** — the shadow appears, disappears, and reappears with no
/// authored reason. The stability fix is [`select_point_casters`]'s
/// hysteresis; this is the half that decides what stability is worth
/// preserving.
///
/// [`select_point_casters`]: https://docs.rs/kooch_render
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

/// The point lights that cast, ranked by what a cube spent on them would
/// show ([`point_shadow_importance`]).
///
/// 🔴 Ranked, NOT in whatever order the query walked. Past
/// [`MAX_POINT_SHADOWS`](crate::MAX_POINT_SHADOWS) a light keeps
/// lighting the scene and stops casting, so *which* lights lose their
/// shadow has to be a decision: archetype order means the lamp that goes
/// shadowless is chosen by when it was spawned, and it changes under the
/// author's feet as the scene is edited.
///
/// The slot each light gets here is the `shadow_slot` written into its
/// `GpuLight`, so both walks have to agree — which is why this is one
/// function and not a filter repeated at the call site.
pub fn shadow_casting_points(
    resources: &Resources,
    camera_position: glam::Vec3,
    limit: usize,
) -> Vec<PointShadowSource> {
    let mut out = extract_point_sources(resources);
    for source in &mut out {
        source.importance = point_shadow_importance(
            source.position,
            source.range,
            source.intensity,
            camera_position,
        );
    }
    // Descending: the most worth showing first. `total_cmp` because a
    // NaN intensity would otherwise make the comparator inconsistent and
    // `sort_by` is allowed to panic on that.
    out.sort_by(|a, b| b.importance.total_cmp(&a.importance));
    out.truncate(limit);
    out
}

fn extract_point_sources(resources: &Resources) -> Vec<PointShadowSource> {
    let mut out = Vec::new();
    Query::<(&PointLight, &GlobalTransform)>::new(resources).for_each_entity(
        |entity, (light, transform)| {
            if light.active && light.cast_shadows {
                out.push(PointShadowSource {
                    entity,
                    position: transform.matrix.w_axis.truncate(),
                    range: light.range,
                    intensity: light.intensity,
                    // Filled by `shadow_casting_points`, which is the
                    // only thing that knows where the camera is.
                    importance: 0.0,
                });
            }
        },
    );
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
                    outer_angle: light.outer_angle.clamp(0.0, 90.0).to_radians(),
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
/// A POINT light has no shadow map: it would need a cube map, which is
/// #778. Contact shadows are the only occlusion it can have, and they
/// are opt-in and off by default — fifty lamps should not each cost a
/// screen-space march.
///
/// A spot light does have one since #777, so its `cast_shadows` means
/// what it says.
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
    // Since #778 a point light has a real cube map, so its
    // `cast_shadows` is finally a promise the engine keeps and this
    // reads it like every other kind. The note used to say the field
    // meant nothing here, which was true and is the sort of thing that
    // outlives the reason for it.
    //
    // ⚠️ What it still cannot say is whether THIS light got one of the
    // `MAX_POINT_SHADOWS` cubes: that is decided per frame by distance
    // to the camera, and the Inspector is not a frame.
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
