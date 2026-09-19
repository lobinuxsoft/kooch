//! Panel state lifted out of `Resources` for the egui pass: the UI closure holds `Resources`
//! immutably, and the panels edit these in place.

use kooch_core::resource::Resources;
use kooch_physics::backend::DebugCategories;
use kooch_render::meshlet::{MeshletDebugMode, MeshletLodSettings};

pub(super) struct Lifted {
    pub(super) debug_mode: MeshletDebugMode,
    pub(super) lod_settings: MeshletLodSettings,
    /// The lights-per-pixel view's top of scale (#817).
    pub(super) lights_hot: kooch_lighting::LightsHot,
    pub(super) cluster_settings: kooch_lighting::ClusterSettings,
    pub(super) specular_floor: kooch_lighting::SpecularFloor,
    pub(super) gizmo_visibility: crate::gizmos::GizmoVisibility,
    /// Only the switches travel; the overlay resource keeps its reusable line buffer.
    pub(super) physics_debug: DebugCategories,
    /// What the panel drew; the metric systems read it next frame to decide whether to pay (#703).
    pub(super) hud_visibility: crate::perf::HudVisibility,
    pub(super) console: crate::panels::console::ConsoleState,
}

impl Lifted {
    pub(super) fn take(resources: &mut Resources) -> Self {
        Self {
            debug_mode: resources.remove::<MeshletDebugMode>().unwrap_or_default(),
            lod_settings: resources.remove::<MeshletLodSettings>().unwrap_or_default(),
            lights_hot: resources
                .remove::<kooch_lighting::LightsHot>()
                .unwrap_or_default(),
            cluster_settings: resources
                .remove::<kooch_lighting::ClusterSettings>()
                .unwrap_or_default(),
            specular_floor: resources
                .remove::<kooch_lighting::SpecularFloor>()
                .unwrap_or_default(),
            gizmo_visibility: resources
                .get::<crate::gizmos::GizmoVisibility>()
                .cloned()
                .unwrap_or_else(crate::gizmos::GizmoVisibility::new),
            physics_debug: resources
                .get::<crate::gizmos::PhysicsDebugOverlay>()
                .map(|overlay| overlay.categories)
                .unwrap_or_default(),
            hud_visibility: resources
                .get::<crate::perf::HudVisibility>()
                .copied()
                .unwrap_or_default(),
            console: resources
                .remove::<crate::panels::console::ConsoleState>()
                .unwrap_or_default(),
        }
    }

    /// The light the single-light view isolates (#743): the selection, since selecting a light
    /// already means "this one" and a second list would have to track the scene.
    pub(super) fn isolated_light(
        &self,
        overlay: &crate::state::EditorOverlay,
    ) -> Option<kooch_ecs::entity::Entity> {
        self.debug_mode
            .needs_selected_light()
            .then(|| overlay.selected_entities.first().copied())
            .flatten()
    }

    /// Back before the viewport passes, so they and the save system see what the panels changed.
    pub(super) fn put_back(
        self,
        resources: &mut Resources,
        light: Option<kooch_ecs::entity::Entity>,
    ) {
        resources.insert(self.gizmo_visibility);
        resources.insert(self.console);
        resources.insert(self.hud_visibility);
        // Created on first use: a host with no physics never grows one.
        match resources.get_mut::<crate::gizmos::PhysicsDebugOverlay>() {
            Some(overlay) => overlay.categories = self.physics_debug,
            None => {
                if self.physics_debug.any() {
                    resources.insert(crate::gizmos::PhysicsDebugOverlay::new(self.physics_debug));
                }
            }
        }
        resources.insert(self.debug_mode);
        resources.insert(self.lod_settings);
        resources.insert(self.lights_hot);
        resources.insert(self.cluster_settings);
        resources.insert(self.specular_floor);
        resources.insert(kooch_lighting::DebugLight(light));
    }
}
