//! [`Visualizer<C>`] trait + [`VisualizerRegistry`] resource.
//!
//! Pattern: same as [`ComponentRegistry`](ome_ecs::component::ComponentRegistry)
//! — register a function per component type, dispatch at runtime via
//! `TypeId`. Visualizers are pure draw-the-component-as-gizmos hooks,
//! invoked once per (entity, component) pair the editor decides to
//! visualize.
//!
//! # Example
//!
//! ```ignore
//! struct HealthBarVisualizer;
//! impl Visualizer<Health> for HealthBarVisualizer {
//!     fn draw(&self, h: &Health, t: &GlobalTransform, g: &mut Gizmos<'_>) {
//!         let pos = t.translation() + Vec3::Y * 2.0;
//!         g.line(pos, pos + Vec3::X * h.percent(), Vec3::new(0.0, 1.0, 0.0));
//!     }
//! }
//!
//! fn register_visualizers(resources: &mut Resources) {
//!     if let Some(reg) = resources.get_mut::<VisualizerRegistry>() {
//!         reg.register::<Health, HealthBarVisualizer>();
//!     }
//! }
//! ```

use std::any::TypeId;
use std::collections::HashMap;

use ome_core::resource::Resources;
use ome_ecs::component::Component;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;

use crate::gizmos::Gizmos;

/// Trait users implement to draw gizmos for one entity instance of a
/// component type.
pub trait Visualizer<C: Component>: Send + Sync + 'static {
    /// Called once per frame per entity that the editor / runtime
    /// decides to visualize. `transform` is the entity's
    /// world-space [`GlobalTransform`].
    fn draw(&self, component: &C, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>);
}

/// Type-erased dispatch closure stored in the registry. Captures the
/// concrete component type `C` and the visualizer instance, queries
/// the entity, and forwards to the visualizer's `draw`.
type DispatchFn = Box<dyn Fn(Entity, &Resources, &mut Gizmos<'_>) + Send + Sync>;

/// Registry of visualizers, keyed by component `TypeId`. Stored as a
/// `Resources` entry, populated by editor / engine startup, queried
/// each frame by the gizmo system.
#[derive(Default)]
pub struct VisualizerRegistry {
    entries: HashMap<TypeId, DispatchFn>,
}

impl VisualizerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `V` as the visualizer for component type `C`. If a
    /// visualizer is already registered for `C`, it is replaced.
    pub fn register<C, V>(&mut self)
    where
        C: Component,
        V: Visualizer<C> + Default,
    {
        let visualizer = V::default();
        let dispatch: DispatchFn = Box::new(move |entity, resources, gizmos| {
            let query = Query::<(&C, &GlobalTransform)>::new(resources);
            if let Some((component, transform)) = query.get(entity) {
                visualizer.draw(component, transform, gizmos);
            }
        });
        self.entries.insert(TypeId::of::<C>(), dispatch);
    }

    /// Returns `true` if a visualizer is registered for `C`.
    pub fn has<C: Component>(&self) -> bool {
        self.entries.contains_key(&TypeId::of::<C>())
    }

    /// Returns `true` if a visualizer is registered for the given
    /// `TypeId`. Used by the editor's gizmo system.
    pub fn has_type(&self, type_id: TypeId) -> bool {
        self.entries.contains_key(&type_id)
    }

    /// Invokes the registered visualizer for `type_id` against `entity`.
    /// No-op if no visualizer is registered or the entity does not
    /// have the component.
    pub fn dispatch(
        &self,
        type_id: TypeId,
        entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        if let Some(f) = self.entries.get(&type_id) {
            f(entity, resources, gizmos);
        }
    }

    /// Iterates over registered component `TypeId`s.
    pub fn registered_types(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.entries.keys().copied()
    }
}
