//! egui_dock TabViewer implementation for the editor dock area.

use std::collections::HashMap;

use egui_dock::TabViewer;
use glam::Vec3;

use kooch_ecs::entity::Entity;
use kooch_render::meshlet::{
    MeshletDebugCaps, MeshletDebugMode, MeshletLodSettings, MeshletRenderStats,
};

use kooch_gizmos_handles::{HandleMode, SnapSettings};

use crate::actions::EditorAction;
use crate::editor_camera::EditorCameraController;
use crate::editor_camera::input::ViewportInputDelta;
use crate::panels::archetypes::draw_archetypes_content;
use crate::panels::asset_browser::draw_asset_browser_content;
use crate::panels::components::draw_components_content;
use crate::panels::game::draw_game_content;
use crate::panels::inspector::AssetDetail;
use crate::panels::inspector::draw_inspector_content;
use crate::panels::view::draw_view_content;
use crate::panels::world::draw_world_content;
use crate::state::{
    ArchetypeDisplayInfo, ComponentTypeInfo, EditorTab, EntityDisplayInfo, EulerCacheKey,
    ReflectedTypeInfo, RotationDisplayMode,
};

pub(crate) struct EditorTabViewer<'a> {
    /// Which panel the keyboard belongs to, updated here as panels are
    /// drawn. `None` before the user has clicked anything.
    pub(crate) focused_tab: &'a mut Option<EditorTab>,
    /// The colour a focused panel is lit with, read from the theme so it
    /// follows it rather than being a second opinion about it.
    pub(crate) accent: egui::Color32,
    /// The Asset Browser's keyboard cursor and the rows it walks.
    pub(crate) asset_nav: &'a mut crate::panels::asset_browser::AssetNav,
    /// The Inspector's cursor over component sections.
    pub(crate) inspector_nav: &'a mut crate::panels::inspector::InspectorNav,
    pub(crate) entities: &'a [EntityDisplayInfo],
    pub(crate) scenes: &'a [crate::state::SceneDisplayInfo],
    pub(crate) archetypes: &'a [ArchetypeDisplayInfo],
    pub(crate) component_types: &'a [ComponentTypeInfo],
    pub(crate) selected: &'a mut Vec<Entity>,
    /// Entities whose gizmos stay drawn while something else is selected.
    pub(crate) pinned: &'a mut std::collections::HashSet<Entity>,
    pub(crate) reflected_types: &'a [ReflectedTypeInfo],
    pub(crate) actions: &'a mut Vec<EditorAction>,
    pub(crate) entity_count: usize,
    pub(crate) archetype_count: usize,
    pub(crate) active_archetype_count: usize,
    pub(crate) last_clicked_index: &'a mut Option<usize>,
    pub(crate) viewport_texture_id: egui::TextureId,
    pub(crate) viewport_request: &'a mut Option<(u32, u32)>,
    /// Game panel's offscreen texture — a second view of the same
    /// stage, through the gameplay camera (#592).
    pub(crate) game_texture_id: egui::TextureId,
    pub(crate) game_request: &'a mut Option<(u32, u32)>,
    /// Whether the last frame found a gameplay camera to render.
    pub(crate) game_has_camera: bool,
    /// Set while drawing when Game is the focused tab. Drives whether
    /// the project receives input — a key pressed with the World panel
    /// selected is an editor shortcut, not a jump.
    /// This frame's input owner, resolved once (see [`crate::input_focus`])
    /// and read by every consumer instead of each re-deriving it.
    pub(crate) input_owner: &'a mut crate::input_focus::InputOwner,
    pub(crate) viewport_input: &'a mut Option<ViewportInputDelta>,
    pub(crate) editor_camera_controller: &'a EditorCameraController,
    pub(crate) rotation_euler_cache: &'a mut HashMap<EulerCacheKey, Vec3>,
    pub(crate) rotation_display_mode: &'a mut RotationDisplayMode,
    pub(crate) snap_settings: &'a mut SnapSettings,
    pub(crate) handle_mode: HandleMode,
    /// `true` when at least one currently-selected entity carries a
    /// `Transform` — gates the viewport's Local/World toggle.
    pub(crate) selection_has_transform: bool,
    /// Which gizmo groups draw. Threaded through so the Gizmos dropdown
    /// can mutate it from the viewport toolbar.
    pub(crate) gizmo_visibility: &'a mut crate::gizmos::GizmoVisibility,
    pub(crate) physics_debug: &'a mut kooch_physics::backend::DebugCategories,
    /// Whether anyone is looking at the metrics that cost to take (#703).
    pub(crate) hud_visibility: &'a mut crate::perf::HudVisibility,
    pub(crate) log_buffer: Option<&'a kooch_core::LogBuffer>,
    pub(crate) console: &'a mut crate::panels::console::ConsoleState,
    /// The registered visualizers, grouped by category, for that dropdown.
    pub(crate) gizmo_groups: &'a [crate::gizmos::GizmoGroup],
    /// Per-frame snapshot of the `AssetDatabase` consumed by the
    /// inspector's typed asset picker.
    pub(crate) asset_catalog: &'a [crate::panels::inspector::AssetCatalogEntry],
    /// Asset Browser selection (owned by the overlay). Row clicks mutate
    /// it; the render system reads it to pre-resolve `asset_detail`.
    pub(crate) selected_asset: &'a mut Option<kooch_core::Guid>,
    /// What the Build panel draws: the presets, the running job's status
    /// and cargo's output (#758).
    pub(crate) build: &'a crate::panels::build::BuildPanel,
    /// Which preset the Build panel has selected. Separate from
    /// `selected_asset`: choosing a preset shows it in the Inspector, but
    /// selecting something else there must not change what Build builds.
    pub(crate) build_selection: &'a mut Option<kooch_core::Guid>,
    /// Data snapshot for the selected asset, resolved before the frame.
    /// `None` when nothing is selected or the snapshot is still pending.
    pub(crate) asset_detail: Option<&'a AssetDetail>,
    /// The `.inputmap` open in the Input Map panel, if any.
    pub(crate) open_input_map: Option<&'a crate::state::OpenInputMap>,
    /// Asset Browser folder selection — the drag-and-drop import target.
    pub(crate) current_folder: &'a mut Option<std::path::PathBuf>,
    /// Project / engine `assets/` roots, for the Asset Browser tree.
    pub(crate) engine_assets_root: Option<&'a std::path::Path>,
    pub(crate) project_assets_root: Option<&'a std::path::Path>,
    /// The scene the project opens with, for the Asset Browser to mark
    /// (#808). Resolved once per frame from the manifest.
    pub(crate) main_scene: Option<&'a std::path::Path>,
    /// Selector for the meshlet pipeline's debug visualization
    /// (#451). Mutated by the View toolbar dropdown.
    pub(crate) meshlet_debug_mode: &'a mut MeshletDebugMode,
    /// Capability probe (#454). Decides which debug modes the View /
    /// Performance dropdowns surface based on the device's
    /// `Features::TEXTURE_ATOMIC` exposure.
    pub(crate) meshlet_debug_caps: MeshletDebugCaps,
    /// What the light isolated by `SingleLight` actually casts (#743).
    pub(crate) single_light_note: Option<&'a str>,
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

    /// Lights the focused panel's own border, rather than drawing a line
    /// inside its contents.
    ///
    /// The first attempt painted a stroke on the body rect from in here,
    /// and it landed a few pixels inside the border egui_dock had already
    /// drawn — so instead of the panel standing out there were two lines,
    /// one of them crooked. This hands egui_dock a brighter stroke and
    /// lets it draw in the place it was going to draw anyway, which is the
    /// only way the alignment is right by construction.
    ///
    /// The tab's title is lit too: it is the part a user's eye is already
    /// using to tell panels apart.
    fn tab_style_override(
        &self,
        tab: &Self::Tab,
        global: &egui_dock::TabStyle,
    ) -> Option<egui_dock::TabStyle> {
        if *self.focused_tab != Some(*tab) {
            return None;
        }
        let mut style = global.clone();
        style.tab_body.stroke.color = self.accent;
        // Hairlines disappear at a bright colour as readily as at a dim
        // one; a focused border has to survive being looked at.
        style.tab_body.stroke.width = style.tab_body.stroke.width.max(1.0);
        for interaction in [
            &mut style.active,
            &mut style.focused,
            &mut style.active_with_kb_focus,
            &mut style.focused_with_kb_focus,
        ] {
            interaction.outline_color = self.accent;
        }
        Some(style)
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        // One place decides focus for every panel, including View.
        //
        // Any pointer press inside the body counts, left or right. That is
        // deliberate: right-drag already means "fly the camera", so
        // requiring a left click would make the viewport the one panel you
        // cannot focus with the gesture you actually use — and starting the
        // drag inside is what keeps focus from being lost mid-flight
        // (#661).
        let body = ui.available_rect_before_wrap();
        let pressed_here = ui.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|pos| body.contains(pos))
        });
        if pressed_here {
            *self.focused_tab = Some(*tab);
        }
        let focused = *self.focused_tab == Some(*tab);
        // Resolved here because this is where panel focus is known, and
        // resolved once: consumers ask `belongs_to`, they do not rebuild
        // the rule.
        // `text_edit_focused`, NOT `egui_wants_keyboard_input`: the
        // latter is `memory.focused().is_some()` despite its name, so any
        // focused button would silently take the keyboard from the View.
        *self.input_owner =
            crate::input_focus::resolve(*self.focused_tab, ui.ctx().text_edit_focused());

        // A cursor left lit on a panel that no longer owns the keyboard is
        // a highlight that means nothing: it says "the arrows go here" when
        // they do not. Clearing it on focus loss is the whole fix.
        //
        // Only the *keyboard* cursors. The World panel's entity selection
        // is not one of these — you pick an entity there and then edit it
        // in the Inspector, so clearing that would break the one workflow
        // the editor is for.
        if !focused {
            match tab {
                EditorTab::AssetBrowser => self.asset_nav.cursor = None,
                EditorTab::Inspector => self.inspector_nav.cursor = None,
                EditorTab::Console => self.console.clear_cursor(),
                _ => {}
            }
        }

        match tab {
            EditorTab::World => draw_world_content(
                ui,
                focused,
                self.entities,
                self.selected,
                self.pinned,
                self.reflected_types,
                self.actions,
                self.entity_count,
                self.archetype_count,
                self.active_archetype_count,
                self.last_clicked_index,
                self.scenes,
            ),
            EditorTab::Game => draw_game_content(
                ui,
                self.game_texture_id,
                self.game_request,
                self.game_has_camera,
                self.perf_stats,
                self.meshlet_stats,
                self.meshlet_debug_mode,
                self.meshlet_debug_caps,
                self.single_light_note,
                self.meshlet_lod_settings,
                self.hud_visibility,
            ),
            EditorTab::View => draw_view_content(
                ui,
                *self.input_owner == crate::input_focus::InputOwner::ViewCamera,
                self.viewport_texture_id,
                self.viewport_request,
                self.viewport_input,
                self.editor_camera_controller,
                self.handle_mode,
                self.rotation_display_mode,
                self.snap_settings,
                self.selection_has_transform,
                self.gizmo_visibility,
                self.gizmo_groups,
                self.physics_debug,
                self.actions,
            ),
            EditorTab::Console => {
                crate::panels::console::draw_console(ui, focused, self.log_buffer, self.console)
            }
            EditorTab::Inspector => draw_inspector_content(
                ui,
                focused,
                self.inspector_nav,
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
            // The map and the live values are not plumbed through yet —
            // the panel already says what to do with no map open, which
            // is the honest state until the asset handle reaches here.
            EditorTab::InputMap => {
                let requested = crate::panels::input_map::draw_input_map_content(
                    ui,
                    crate::panels::input_map::InputMapView {
                        map: self.open_input_map.map(|open| &open.map),
                        // Live values arrive over the protocol from the
                        // host, which is the only process that simulates.
                        live: &[],
                        awaiting: None,
                        dirty: self.open_input_map.is_some_and(|open| open.dirty),
                        selected: self.open_input_map.and_then(|open| open.selected),
                        single_action: self.open_input_map.is_some_and(|open| {
                            open.kind == crate::state::OpenInputKind::SingleAction
                        }),
                    },
                );
                for request in requested {
                    self.actions.push(match request {
                        crate::panels::input_map::InputMapAction::Save => {
                            crate::actions::EditorAction::SaveInputMap
                        }
                        edit => crate::actions::EditorAction::EditInputMap(edit),
                    });
                }
            }
            EditorTab::Components => draw_components_content(ui, self.component_types),
            EditorTab::Profiler => crate::panels::profiler::draw_profiler_content(ui),
            EditorTab::Build => crate::panels::build::draw_build_content(
                ui,
                self.build,
                self.build_selection,
                self.selected_asset,
                self.actions,
            ),
            EditorTab::AssetBrowser => draw_asset_browser_content(
                ui,
                focused,
                self.asset_nav,
                self.asset_catalog,
                self.selected_asset,
                self.current_folder,
                self.engine_assets_root,
                self.project_assets_root,
                self.main_scene,
                self.actions,
            ),
        }
    }

    fn closeable(&mut self, _tab: &mut Self::Tab) -> bool {
        true
    }
}
