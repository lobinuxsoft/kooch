//! Reads a scene and reports which way it pulls at every dynamic body.
//!
//! Throwaway: a hand-written scene claims things about direction that
//! nothing else checks until it is opened in the editor.

use glam::Vec3;

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::scene::{ComponentDescription, SceneDocument};
use kooch_ecs::transform::Transform;
use kooch_gravity::{
    AreaGravity, BoxGravity, GlobalGravity, GravityPriority, PlaneGravity, PointGravity, gravity_at,
};

fn f32_of(c: &ComponentDescription, key: &str) -> f32 {
    c.fields
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, v)| match v {
            ReflectValue::F32(x) => Some(*x),
            _ => None,
        })
        .unwrap_or_default()
}

fn vec3_of(c: &ComponentDescription, key: &str) -> Vec3 {
    c.fields
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, v)| match v {
            ReflectValue::Vec3(x) => Some(*x),
            _ => None,
        })
        .unwrap_or_default()
}

fn u32_of(c: &ComponentDescription, key: &str) -> u32 {
    c.fields
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, v)| match v {
            ReflectValue::U32(x) => Some(*x),
            _ => None,
        })
        .unwrap_or_default()
}

fn i32_of(c: &ComponentDescription, key: &str) -> i32 {
    c.fields
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, v)| match v {
            ReflectValue::I32(x) => Some(*x),
            _ => None,
        })
        .unwrap_or_default()
}

fn bool_of(c: &ComponentDescription, key: &str) -> bool {
    c.fields
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, v)| match v {
            ReflectValue::Bool(x) => Some(*x),
            _ => None,
        })
        .unwrap_or_default()
}

fn put<T: Component>(resources: &mut Resources, entity: Entity, value: T) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<T>()
    {
        storage.insert(entity, value);
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: check_scene <path>");
    let doc = SceneDocument::load(std::path::Path::new(&path)).expect("parse");

    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<GlobalGravity>();
    registry.register_cpu_reflected::<PointGravity>();
    registry.register_cpu_reflected::<AreaGravity>();
    registry.register_cpu_reflected::<BoxGravity>();
    registry.register_cpu_reflected::<PlaneGravity>();
    registry.register_cpu_reflected::<GravityPriority>();

    let mut bodies: Vec<(String, Vec3)> = Vec::new();

    for described in &doc.entities {
        let mut allocator = resources.remove::<EntityAllocator>().unwrap();
        let entity = allocator.spawn();
        resources.insert(allocator);

        let mut position = Vec3::ZERO;
        let mut rotation = glam::Quat::IDENTITY;
        let mut scale = Vec3::ONE;
        let mut dynamic = false;

        for component in &described.components {
            match component.type_name.rsplit("::").next().unwrap_or_default() {
                "Transform" => {
                    position = vec3_of(component, "position");
                    scale = vec3_of(component, "scale");
                    if let Some((_, ReflectValue::Quat(q))) =
                        component.fields.iter().find(|(n, _)| n == "rotation")
                    {
                        rotation = *q;
                    }
                }
                "PhysicsBody" => dynamic = u32_of(component, "kind") == 0,
                "GlobalGravity" => put(
                    &mut resources,
                    entity,
                    GlobalGravity {
                        acceleration: vec3_of(component, "acceleration"),
                    },
                ),
                "PointGravity" => put(
                    &mut resources,
                    entity,
                    PointGravity {
                        strength: f32_of(component, "strength"),
                        radius: f32_of(component, "radius"),
                        range: f32_of(component, "range"),
                        inverse_square: bool_of(component, "inverse_square"),
                    },
                ),
                "AreaGravity" => put(
                    &mut resources,
                    entity,
                    AreaGravity {
                        direction: vec3_of(component, "direction"),
                        strength: f32_of(component, "strength"),
                        half_extents: vec3_of(component, "half_extents"),
                        falloff: f32_of(component, "falloff"),
                    },
                ),
                "BoxGravity" => put(
                    &mut resources,
                    entity,
                    BoxGravity {
                        half_extents: vec3_of(component, "half_extents"),
                        strength: f32_of(component, "strength"),
                        rounding: f32_of(component, "rounding"),
                        range: f32_of(component, "range"),
                        falloff: f32_of(component, "falloff"),
                    },
                ),
                "PlaneGravity" => put(
                    &mut resources,
                    entity,
                    PlaneGravity {
                        normal: vec3_of(component, "normal"),
                        strength: f32_of(component, "strength"),
                        range: f32_of(component, "range"),
                        falloff: f32_of(component, "falloff"),
                    },
                ),
                "GravityPriority" => put(
                    &mut resources,
                    entity,
                    GravityPriority {
                        level: i32_of(component, "level"),
                    },
                ),
                _ => {}
            }
        }
        put(
            &mut resources,
            entity,
            Transform {
                position,
                rotation,
                scale,
            },
        );
        if dynamic {
            bodies.push((described.name.clone(), position));
        }
    }

    println!("{} — {} dynamic bodies\n", doc.name, bodies.len());
    for (name, at) in bodies {
        let pull = gravity_at(&resources, at);
        let direction = match pull.try_normalize() {
            Some(unit) => format!("[{:>5.2} {:>5.2} {:>5.2}]", unit.x, unit.y, unit.z),
            None => "  NOTHING REACHES IT  ".to_string(),
        };
        println!("  {name:<28} {direction}  {:>6.2} m/s²", pull.length());
    }
}
