//! One walk of the world's lights, and everything a frame derives from it.

use kooch_core::resource::Resources;
use kooch_ecs::EntityAllocator;
use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::query::Query;
use kooch_ecs::spot_light::SpotLight;

use crate::extract::{ExtractedLights, PointShadowSource, SpotShadowSource};
use crate::gpu_light::GpuLight;

/// Every light of one frame from one walk per archetype — two walks agree only until someone edits
/// one, and nothing fails when they drift.
/// Built per frame and borrowed, never stored, so its entities cannot be despawned ones.
pub struct LightFrame {
    lights: ExtractedLights,
    sun: Option<glam::Vec3>,
    /// The casting sun's own shadow mask (#1220), beside its direction.
    sun_shadow_layers: u32,
    point_shadows: Vec<PointShadowSource>,
    spot_shadows: Vec<SpotShadowSource>,
}

impl LightFrame {
    /// Walks each light archetype once. ⚠️ Skips entities the allocator considers dead: despawn is
    /// deferred.
    pub fn extract(resources: &Resources) -> Self {
        let alive = resources.get::<EntityAllocator>();
        let live = |entity: Entity| alive.is_none_or(|alloc| alloc.is_alive(entity));

        let mut lights = Vec::new();
        let mut entities = Vec::new();
        let mut sun = None;
        let mut sun_shadow_layers = u32::MAX;
        let mut point_shadows = Vec::new();
        let mut spot_shadows = Vec::new();

        Query::<(&DirectionalLight, &GlobalTransform)>::new(resources).for_each_entity(
            |entity, (light, transform)| {
                if !live(entity) || !light.active {
                    return;
                }
                // The first casting sun: the atlas holds four cascades of one light.
                if sun.is_none() && light.cast_shadows {
                    sun = Some(crate::gpu_light::forward(transform.matrix));
                    sun_shadow_layers = light.shadow_layers;
                }
                lights.push(GpuLight::directional(light, transform.matrix));
                entities.push(entity);
            },
        );

        // Everything pushed above is directional and everything below is
        // not. `directional_count` is that boundary, and the shading loop
        // reads it as one.
        let directional_count = lights.len() as u32;

        Query::<(&PointLight, &GlobalTransform)>::new(resources).for_each_entity(
            |entity, (light, transform)| {
                if !live(entity) || !light.active {
                    return;
                }
                if light.cast_shadows {
                    point_shadows.push(PointShadowSource {
                        entity,
                        buffer_slot: lights.len() as u32,
                        position: transform.matrix.w_axis.truncate(),
                        range: light.range,
                        intensity: light.intensity,
                        // Filled by `ranked_points`, which is the only
                        // thing that knows where the camera is.
                        importance: 0.0,
                        shadow_layers: light.shadow_layers,
                    });
                }
                lights.push(GpuLight::point(light, transform.matrix));
                entities.push(entity);
            },
        );

        // 🔴 Spot slots are assigned during the walk, with the budget applied only here.
        let mut next_slot = 0u32;
        Query::<(&SpotLight, &GlobalTransform)>::new(resources).for_each_entity(
            |entity, (light, transform)| {
                if !live(entity) || !light.active {
                    return;
                }
                let mut gpu = GpuLight::spot(light, transform.matrix);
                if light.cast_shadows && (next_slot as usize) < crate::MAX_SPOT_SHADOWS {
                    gpu.shadow_slot = next_slot;
                    next_slot += 1;
                    spot_shadows.push(SpotShadowSource {
                        entity,
                        buffer_slot: lights.len() as u32,
                        position: transform.matrix.w_axis.truncate(),
                        direction: crate::gpu_light::forward(transform.matrix),
                        outer_angle: light.outer_angle.clamp(0.0, 90.0).to_radians(),
                        range: light.range,
                        shadow_layers: light.shadow_layers,
                    });
                }
                lights.push(gpu);
                entities.push(entity);
            },
        );

        Self {
            lights: ExtractedLights {
                lights,
                entities,
                directional_count,
            },
            sun,
            sun_shadow_layers,
            point_shadows,
            spot_shadows,
        }
    }

    /// The lights as the shader reads them.
    #[inline]
    pub fn lights(&self) -> &ExtractedLights {
        &self.lights
    }

    /// The lights, mutably — the shadow stage writes cube slots back.
    #[inline]
    pub fn lights_mut(&mut self) -> &mut ExtractedLights {
        &mut self.lights
    }

    /// What casts into the sun's shadow (#1220). Every layer when the scene has no casting sun.
    #[inline]
    pub fn sun_shadow_layers(&self) -> u32 {
        self.sun_shadow_layers
    }

    /// Where the one shadow-casting sun shines, if the scene has one.
    #[inline]
    pub fn sun(&self) -> Option<glam::Vec3> {
        self.sun
    }

    /// The spot lights that cast, already capped at the shadow budget and
    /// numbered in the order their slots were handed out.
    #[inline]
    pub fn spot_shadows(&self) -> &[SpotShadowSource] {
        &self.spot_shadows
    }

    /// The point lights that cast, unranked.
    #[inline]
    pub fn point_shadows(&self) -> &[PointShadowSource] {
        &self.point_shadows
    }

    /// Casting point lights ranked by what a cube would show from `camera`, best first, cut to
    /// `limit` — here, since only ranking depends on position.
    pub fn ranked_points(&self, camera: glam::Vec3, limit: usize) -> Vec<PointShadowSource> {
        let mut out = self.point_shadows.clone();
        for source in &mut out {
            source.importance = crate::extract::point_shadow_importance(
                source.position,
                source.range,
                source.intensity,
                camera,
            );
        }
        // Descending. `total_cmp` because a NaN intensity would otherwise
        // make the comparator inconsistent and `sort_by` may panic on that.
        out.sort_by(|a, b| b.importance.total_cmp(&a.importance));
        out.truncate(limit);
        out
    }
}

#[cfg(test)]
mod tests;
