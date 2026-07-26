//! Editor overlay types and state.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui_dock::{DockState, NodeIndex};
use glam::Vec3;
use winit::event::WindowEvent;
use winit::window::Window;

use ome_core::raw_event::RawEventHandler;
use ome_ecs::component::ComponentId;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::{FieldMeta, InspectorVisibility, ReflectValue};
use ome_ecs::transform::Transform;
use ome_gizmos_handles::SnapSettings;

/// Shared egui-winit state for event forwarding between the
/// window event handler and the render system.
pub(crate) type SharedWinitState = Arc<Mutex<egui_winit::State>>;

// ---------------------------------------------------------------------------
// Dock tabs
// ---------------------------------------------------------------------------

/// Identifiers for each dockable editor tab.
///
/// `Serialize`/`Deserialize` enable persisting the dock layout between
/// editor sessions via [`crate::layout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum EditorTab {
    World,
    View,
    Inspector,
    Archetypes,
    Components,
    AssetBrowser,
}

/// All tab variants, used for the Window menu.
pub(crate) const ALL_TABS: &[EditorTab] = &[
    EditorTab::World,
    EditorTab::View,
    EditorTab::Inspector,
    EditorTab::Archetypes,
    EditorTab::Components,
    EditorTab::AssetBrowser,
];

impl EditorTab {
    /// Returns the display label with icon.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::World => format!("{} World", crate::icons::GLOBE),
            Self::View => format!("{} View", crate::icons::EYE),
            Self::Inspector => format!("{} Inspector", crate::icons::SLIDERS),
            Self::Archetypes => format!("{} Archetypes", crate::icons::TREE_STRUCTURE),
            Self::Components => format!("{} Components", crate::icons::LIST_BULLETS),
            Self::AssetBrowser => format!("{} Assets", crate::icons::FOLDER_OPEN),
        }
    }
}

impl std::fmt::Display for EditorTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Creates the default 3-panel dock layout: World | View | Inspector.
/// Per-frame performance metrics are not a dock tab — the View panel
/// renders them as a vertical overlay anchored to its right edge, so
/// they are always visible alongside what the artist is looking at.
pub(crate) fn default_dock_state() -> DockState<EditorTab> {
    let mut state = DockState::new(vec![EditorTab::View]);

    let surface = state.main_surface_mut();
    surface.split_left(NodeIndex::root(), 0.2, vec![EditorTab::World]);

    let surface = state.main_surface_mut();
    surface.split_right(NodeIndex::root(), 0.7, vec![EditorTab::Inspector]);

    state
}

/// Returns `true` if the given tab exists anywhere in the dock state.
pub(crate) fn dock_has_tab(dock_state: &DockState<EditorTab>, tab: &EditorTab) -> bool {
    dock_state.iter_all_tabs().any(|(_, t)| t == tab)
}

// ---------------------------------------------------------------------------
// Editor overlay resource
// ---------------------------------------------------------------------------

/// Display mode for `Transform.rotation` in the Inspector panel.
///
/// `Local` (the default) shows the rotation stored in the Transform
/// directly, i.e. relative to the entity's parent. `World` shows the
/// world-space rotation computed by the hierarchy propagation, and
/// converts user edits back to local on write. The Transform storage
/// itself never changes representation — only the display does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum RotationDisplayMode {
    #[default]
    Local,
    World,
}

/// Cache key for the editor's per-field Euler rotation state.
///
/// Scoped by `(Entity, component TypeId, field name, display mode)` so
/// different Quat fields — or the same field under different display
/// modes — do not collide.
pub(crate) type EulerCacheKey = (Entity, TypeId, String, RotationDisplayMode);

/// Editor overlay state, stored as a resource.
///
/// Holds the egui context, winit integration state, wgpu renderer,
/// dock layout, and UI state (entity selection).
pub struct EditorOverlay {
    pub(crate) ctx: egui::Context,
    pub(crate) winit_state: SharedWinitState,
    pub(crate) renderer: egui_wgpu::Renderer,
    pub(crate) dock_state: DockState<EditorTab>,
    pub(crate) selected_entities: Vec<Entity>,
    /// Anchor index for Shift+Click range selection in the World panel.
    pub(crate) last_clicked_index: Option<usize>,
    /// Per-field Euler angle cache (radians, XYZ convention) for Quat
    /// rotation fields. Kept to avoid a `Quat → Euler → Quat` round-trip
    /// every frame, which introduces gimbal lock when crossing ±90° on
    /// any axis. See issue #202.
    pub(crate) rotation_euler_cache: HashMap<EulerCacheKey, Vec3>,
    /// Display mode for `Transform.rotation` in the Inspector. Toggled
    /// via a button in the Inspector header. Persists for the session.
    pub(crate) rotation_display_mode: RotationDisplayMode,
    /// User-tunable snap step sizes for the gizmo handles. Edited from
    /// the viewport toolbar.
    pub(crate) snap_settings: SnapSettings,
    /// Snapshot of the entity's `Transform` at the moment a viewport
    /// gizmo drag started. `Some` while a drag is in progress, `None`
    /// otherwise. Used to emit a single `TransformEdit` undo entry per
    /// drag (instead of one per frame) when the user releases.
    pub(crate) gizmo_drag_start: Option<(Entity, Transform)>,
    /// Asset selected in the Asset Browser panel, by `Guid`. Drives the
    /// Inspector's asset view. Held on the overlay (not egui temp state)
    /// so the render system can resolve the asset's data snapshot before
    /// the egui frame runs.
    pub(crate) selected_asset: Option<ome_core::Guid>,
    /// Folder selected in the Asset Browser tree — the destination for
    /// drag-and-drop imports. `None` falls back to the project assets
    /// root. Only project folders are valid targets (engine is read-only).
    pub(crate) current_folder: Option<std::path::PathBuf>,
}

/// Forwards raw winit events to egui for input processing.
pub(crate) struct EguiEventHandler {
    pub(crate) winit_state: SharedWinitState,
}

impl RawEventHandler for EguiEventHandler {
    fn on_event(&mut self, window: &dyn Any, event: &dyn Any) -> bool {
        let Some(window) = window.downcast_ref::<Window>() else {
            return false;
        };
        let Some(event) = event.downcast_ref::<WindowEvent>() else {
            return false;
        };
        let mut state = self.winit_state.lock().unwrap();
        state.on_window_event(window, event).consumed
    }
}

// ---------------------------------------------------------------------------
// Display data (gathered before egui frame)
// ---------------------------------------------------------------------------

/// Display data for a single component on an entity.
pub(crate) struct ComponentDisplayInfo {
    /// Local type handle, used for reflection and egui id salts. Absent
    /// on a remote client that has no Rust type for this component.
    pub(crate) type_id: TypeId,
    /// Portable identity, carried by any action this component emits.
    pub(crate) component: ComponentId,
    pub(crate) short_name: String,
    pub(crate) fields: Option<Vec<(String, ReflectValue)>>,
    /// Static field metadata parallel to `fields`. Used to pick widget
    /// kinds (e.g. dropdown for `choices`) without re-querying the
    /// ComponentRegistry during the UI pass.
    pub(crate) field_metas: Option<&'static [FieldMeta]>,
    pub(crate) visibility: InspectorVisibility,
}

/// One open scene, as the World panel needs to show it.
///
/// A snapshot rather than a borrow of `SceneManager`: the UI pass runs
/// while `Resources` is borrowed elsewhere, which is the same reason
/// [`EntityDisplayInfo`] exists.
#[derive(Debug, Clone)]
pub(crate) struct SceneDisplayInfo {
    pub(crate) id: ome_core::Guid,
    /// File stem, or "Untitled" for a scene never saved.
    pub(crate) name: String,
    pub(crate) dirty: bool,
    pub(crate) active: bool,
}

pub(crate) struct EntityDisplayInfo {
    pub(crate) entity: Entity,
    pub(crate) components: Vec<ComponentDisplayInfo>,
    /// Parent entity, if any.
    pub(crate) parent: Option<Entity>,
    /// Direct child entities.
    pub(crate) children: Vec<Entity>,
    /// Depth in the hierarchy tree (0 = root).
    pub(crate) depth: usize,
    /// World-space rotation from `GlobalTransform`, if available. Used
    /// by the Inspector's World rotation display mode.
    pub(crate) global_rotation: Option<glam::Quat>,
    /// Scene this entity was authored in, or `None` for one that belongs
    /// to no scene — an editor helper, or something spawned but not yet
    /// saved into any file.
    pub(crate) scene: Option<ome_core::Guid>,
    /// Parent's world-space rotation from `GlobalTransform`, if the
    /// entity has a parent and that parent has a `GlobalTransform`.
    /// Used to convert World-space edits back to the local rotation
    /// stored on the entity's own Transform.
    pub(crate) parent_global_rotation: Option<glam::Quat>,
}

/// Display data for a single archetype.
pub(crate) struct ArchetypeDisplayInfo {
    pub(crate) id_short: String,
    pub(crate) entity_count: usize,
    pub(crate) component_names: Vec<String>,
}

/// Display data for a registered component type.
pub(crate) struct ComponentTypeInfo {
    /// Portable identity — how the UI keys this type and what it carries
    /// when dragged onto an entity.
    pub(crate) component: ComponentId,
    pub(crate) short_name: String,
    pub(crate) has_reflection: bool,
}

/// Available reflected component types for "Add Component".
pub(crate) struct ReflectedTypeInfo {
    /// Portable identity, carried by the emitted `AddComponent` action.
    pub(crate) component: ComponentId,
    pub(crate) short_name: String,
    pub(crate) category: Option<String>,
}
