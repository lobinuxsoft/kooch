//! egui_dock TabViewer implementation for the editor dock area.

use std::collections::HashMap;

use egui_dock::TabViewer;
use glam::Vec3;

use ome_ecs::entity::Entity;
use ome_render::meshlet::{
    MeshletDebugCaps, MeshletDebugMode, MeshletLodSettings, MeshletRenderStats,
};

use ome_gizmos_handles::{HandleMode, SnapSettings};

use crate::actions::EditorAction;
use crate::editor_camera::EditorCameraController;
use crate::editor_camera::input::ViewportInputDelta;
use crate::panels::archetypes::draw_archetypes_content;
use crate::panels::asset_browser::draw_asset_browser_content;
use crate::panels::components::draw_components_content;
use crate::panels::inspector::AssetDetail;
use crate::panels::inspector::draw_inspector_content;
use crate::panels::view::draw_view_content;
use crate::panels::world::draw_world_content;
use crate::state::{
    ArchetypeDisplayInfo, ComponentTypeInfo, EditorTab, EntityDisplayInfo, EulerCacheKey,
    ReflectedTypeInfo, RotationDisplayMode,
};

pub(crate) struct EditorTabViewer<'a> {
    pub(crate) entities: &'a [EntityDisplayInfo],
    pub(crate) archetypes: &'a [ArchetypeDisplayInfo],
    pub(crate) component_types: &'a [ComponentTypeInfo],
    pub(crate) selected: &'a mut Vec<Entity>,
    pub(crate) reflected_types: &'a [ReflectedTypeInfo],
    pub(crate) actions: &'a mut Vec<EditorAction>,
    pub(crate) entity_count: usize,
    pub(crate) archetype_count: usize,
    pub(crate) active_archetype_count: usize,
    pub(crate) last_clicked_index: &'a mut Option<usize>,
    pub(crate) viewport_texture_id: egui::TextureId,
    pub(crate) viewport_request: &'a mut Option<(u32, u32)>,
    pub(crate) viewport_input: &'a mut Option<ViewportInputDelta>,
    pub(crate) editor_camera_controller: &'a EditorCameraController,
    pub(crate) rotation_euler_cache: &'a mut HashMap<EulerCacheKey, Vec3>,
    pub(crate) rotation_display_mode: &'a mut RotationDisplayMode,
    pub(crate) snap_settings: &'a mut SnapSettings,
    pub(crate) handle_mode: HandleMode,
    /// `true` when at least one currently-selected entity carries a
    /// `Transform` — gates the viewport's Local/World toggle.
    pub(crate) selection_has_transform: bool,
    /// Per-frame snapshot of the `AssetDatabase` consumed by the
    /// inspector's typed asset picker.
    pub(crate) asset_catalog: &'a [crate::panels::inspector::AssetCatalogEntry],
    /// Asset Browser selection (owned by the overlay). Row clicks mutate
    /// it; the render system reads it to pre-resolve `asset_detail`.
    pub(crate) selected_asset: &'a mut Option<ome_core::Guid>,
    /// Data snapshot for the selected asset, resolved before the frame.
    /// `None` when nothing is selected or the snapshot is still pending.
    pub(crate) asset_detail: Option<&'a AssetDetail>,
    /// Asset Browser folder selection — the drag-and-drop import target.
    pub(crate) current_folder: &'a mut Option<std::path::PathBuf>,
    /// Project / engine `assets/` roots, for the Asset Browser tree.
    pub(crate) engine_assets_root: Option<&'a std::path::Path>,
    pub(crate) project_assets_root: Option<&'a std::path::Path>,
    /// Selector for the meshlet pipeline's debug visualization
    /// (#451). Mutated by the View toolbar dropdown.
    pub(crate) meshlet_debug_mode: &'a mut MeshletDebugMode,
    /// Capability probe (#454). Decides which debug modes the View /
    /// Performance dropdowns surface based on the device's
    /// `Features::TEXTURE_ATOMIC` exposure.
    pub(crate) meshlet_debug_caps: MeshletDebugCaps,
    /// Continuous-LOD threshold (#462). Mutated by the View toolbar
    /// slider so artists can sanity-check the chain at editor
    /// distances without rebuilding any pipeline state.
    pub(crate) meshlet_lod_settings: &'a mut MeshletLodSettings,
    /// Per-frame meshlet pipeline counters republished as a Resource by
    /// the viewport render. Read-only, surfaced through the View
    /// toolbar's stats overlay.
    pub(crate) meshlet_stats: MeshletRenderStats,
    /// Per-frame perf HUD counters (#463). Read-only, surfaced
    /// through the View toolbar's perf overlay (always visible).
    pub(crate) perf_stats: crate::perf::EditorPerfStats,
}

impl<'a> TabViewer for EditorTabViewer<'a> {
    type Tab = EditorTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.to_string().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            EditorTab::World => draw_world_content(
                ui,
                self.entities,
                self.selected,
                self.reflected_types,
                self.actions,
                self.entity_count,
                self.archetype_count,
                self.active_archetype_count,
                self.last_clicked_index,
            ),
            EditorTab::View => draw_view_content(
                ui,
                self.viewport_texture_id,
                self.viewport_request,
                self.viewport_input,
                self.editor_camera_controller,
                self.handle_mode,
                self.rotation_display_mode,
                self.snap_settings,
                self.selection_has_transform,
                self.meshlet_debug_mode,
                self.meshlet_debug_caps,
                self.meshlet_lod_settings,
                self.meshlet_stats,
                self.perf_stats,
            ),
            EditorTab::Inspector => draw_inspector_content(
                ui,
                self.entities,
                self.selected,
                self.reflected_types,
                self.actions,
                self.rotation_euler_cache,
                self.rotation_display_mode,
                self.asset_catalog,
                *self.selected_asset,
                self.asset_detail,
            ),
            EditorTab::Archetypes => draw_archetypes_content(ui, self.archetypes),
            EditorTab::Components => draw_components_content(ui, self.component_types),
            EditorTab::AssetBrowser => draw_asset_browser_content(
                ui,
                self.asset_catalog,
                self.selected_asset,
                self.current_folder,
                self.engine_assets_root,
                self.project_assets_root,
                self.actions,
            ),
        }
    }

    fn closeable(&mut self, _tab: &mut Self::Tab) -> bool {
        true
    }
}
