//! Editor overlay types and state.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui_dock::{DockState, NodeIndex};
use glam::Vec3;
use winit::event::WindowEvent;
use winit::window::Window;

use kooch_core::raw_event::RawEventHandler;
use kooch_ecs::component::ComponentId;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::{FieldMeta, InspectorVisibility, ReflectValue};
use kooch_ecs::transform::Transform;
use kooch_gizmos_handles::SnapSettings;

/// Shared egui-winit state for event forwarding between the
/// window event handler and the render system.
pub(crate) type SharedWinitState = Arc<Mutex<egui_winit::State>>;

// ---------------------------------------------------------------------------
// Dock tabs
// ---------------------------------------------------------------------------

/// Identifiers for each dockable editor tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum EditorTab {
    World,
    View,
    /// The scene through the gameplay camera, beside View rather than
    /// instead of it (#592).
    Game,
    Inspector,
    Archetypes,
    Components,
    AssetBrowser,
    InputMap,
    /// The shader graph, which owns the `.shader` it generates (#1159).
    ShaderGraph,
    Console,
    /// Making a shipped game out of the project (#758).
    Build,
    /// Where the frame actually goes (#785).
    Profiler,
    /// The performance metrics as a REAL dock tab (#942-class ask from the user): the overlay
    /// sidebar drew translucent over the game view and could not be read.
    Performance,
    /// What runs each frame, and what is switched off (#982).
    Systems,
}

/// The `.shader` open in the Shader Graph panel, as its graph.
///
/// 🔴 The graph, not the file: the panel edits it and the file is generated from it on save. Cloned
/// into the dock each frame, like the input map — `egui-snarl` edits the graph while it draws it.
#[derive(Clone)]
pub(crate) struct OpenShaderGraph {
    pub path: std::path::PathBuf,
    pub graph: crate::shader_graph::Graph,
    /// Its groups and notes (#1211).
    pub annotations: crate::shader_graph::annotations::Annotations,
    /// Set when the panel should be brought to the front. Cleared by the dock once it has.
    pub focus_requested: bool,
    /// Whether the graph diverges from the file it was read from.
    pub dirty: bool,
}

/// The `.inputmap` currently open in the Input Map panel.
///
/// Which kind of file the input panel has open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenInputKind {
    /// A `.inputmap`: several actions that turn on and off together.
    Map,
    /// A `.inputaction`: one action, referenced by a component.
    SingleAction,
}

/// The parsed map rather than a guid: the panel edits it, and going back to the asset server for
/// every frame's draw would mean the edited copy and the loaded one are two values of the same
/// thing — the shape behind every prefab bug in #611.
#[derive(Debug, Clone)]
pub(crate) struct OpenInputMap {
    pub path: std::path::PathBuf,
    /// What is being edited.
    pub kind: OpenInputKind,
    pub map: kooch_input::actions::ActionMap,
    /// Set when the panel should be brought to the front. Cleared by the
    /// dock once it has done so.
    pub focus_requested: bool,
    /// What the properties pane is editing.
    pub selected: Option<crate::panels::input_map::Selection>,
    /// Whether this diverges from what is on disk.
    pub dirty: bool,
}

/// All tab variants, used for the Window menu.
pub(crate) const ALL_TABS: &[EditorTab] = &[
    EditorTab::World,
    EditorTab::View,
    EditorTab::Game,
    EditorTab::Inspector,
    EditorTab::Archetypes,
    EditorTab::Components,
    EditorTab::Console,
    EditorTab::AssetBrowser,
    EditorTab::InputMap,
    EditorTab::ShaderGraph,
    EditorTab::Build,
    EditorTab::Profiler,
    EditorTab::Performance,
    EditorTab::Systems,
];

impl EditorTab {
    /// Returns the display label with icon.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::World => format!("{} World", crate::icons::GLOBE),
            Self::Systems => format!("{} Systems", crate::icons::LIST_BULLETS),
            // "Edit View" / "Game View", the user's naming: both are real views of the same world,
            // one through the authoring camera and one through the gameplay camera.
            Self::View => format!("{} Edit View", crate::icons::EYE),
            Self::Game => format!("{} Game View", crate::icons::GAME_CONTROLLER),
            Self::Inspector => format!("{} Inspector", crate::icons::SLIDERS),
            Self::Archetypes => format!("{} Archetypes", crate::icons::TREE_STRUCTURE),
            Self::Components => format!("{} Components", crate::icons::LIST_BULLETS),
            Self::AssetBrowser => format!("{} Assets", crate::icons::FOLDER_OPEN),
            Self::InputMap => format!("{} Input Map", crate::icons::SLIDERS),
            Self::ShaderGraph => format!("{} Shader Graph", crate::icons::TREE_STRUCTURE),
            Self::Console => format!("{} Console", crate::icons::TERMINAL),
            Self::Build => format!("{} Build", crate::icons::PACKAGE),
            Self::Profiler => format!("{} Profiler", crate::icons::CHART_BAR),
            Self::Performance => format!("{} Performance", crate::icons::FADERS),
        }
    }
}

impl std::fmt::Display for EditorTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Creates the default 3-panel dock layout: World | View + Game | Inspector + Performance.
pub(crate) fn default_dock_state() -> DockState<EditorTab> {
    let mut state = DockState::new(vec![EditorTab::View, EditorTab::Game]);

    let surface = state.main_surface_mut();
    surface.split_left(NodeIndex::root(), 0.2, vec![EditorTab::World]);

    let surface = state.main_surface_mut();
    surface.split_right(
        NodeIndex::root(),
        0.7,
        vec![EditorTab::Inspector, EditorTab::Performance],
    );

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum RotationDisplayMode {
    #[default]
    Local,
    World,
}

/// Cache key for the editor's per-field Euler rotation state.
pub(crate) type EulerCacheKey = (
    Entity,
    kooch_ecs::component::ComponentId,
    String,
    RotationDisplayMode,
);

/// Editor overlay state, stored as a resource.
pub struct EditorOverlay {
    pub(crate) ctx: egui::Context,
    pub(crate) winit_state: SharedWinitState,
    pub(crate) renderer: egui_wgpu::Renderer,
    pub(crate) dock_state: DockState<EditorTab>,
    /// Which panel the keyboard belongs to.
    pub(crate) focused_tab: Option<EditorTab>,
    /// The Asset Browser's keyboard cursor, and the rows the renderer drew
    /// last frame for it to walk.
    pub(crate) asset_nav: crate::panels::asset_browser::AssetNav,
    /// The Inspector's cursor over component sections.
    pub(crate) inspector_nav: crate::panels::inspector::InspectorNav,
    pub(crate) selected_entities: Vec<Entity>,
    /// Whether a click selects an entity or a face of the selected
    /// block. A second axis beside the handle's mode, not a value of it.
    pub(crate) element_mode: crate::block_edit::ElementMode,
    /// A block's corners as they were when a face drag began, so the
    /// history gets one entry for the gesture rather than one a frame.
    pub(crate) shape_drag_start: Option<kooch_blockmesh::BlockMesh>,
    /// Entities whose gizmos draw whether or not they are selected.
    pub(crate) pinned_gizmos: std::collections::HashSet<Entity>,
    /// Anchor index for Shift+Click range selection in the World panel.
    pub(crate) last_clicked_index: Option<usize>,
    /// Per-field Euler angle cache (radians, XYZ convention) for Quat rotation fields. Kept to
    /// avoid a `Quat → Euler → Quat` round-trip every frame, which introduces gimbal lock when
    /// crossing ±90° on any axis. See issue #202.
    pub(crate) rotation_euler_cache: HashMap<EulerCacheKey, Vec3>,
    /// Display mode for `Transform.rotation` in the Inspector. Toggled
    /// via a button in the Inspector header. Persists for the session.
    pub(crate) rotation_display_mode: RotationDisplayMode,
    /// User-tunable snap step sizes for the gizmo handles. Edited from
    /// the viewport toolbar.
    pub(crate) snap_settings: SnapSettings,
    /// Snapshot of the entity's `Transform` at the moment a viewport gizmo drag started. `Some`
    /// while a drag is in progress, `None` otherwise. Used to emit a single `TransformEdit` undo
    /// entry per drag (instead of one per frame) when the user releases.
    pub(crate) gizmo_drag_start: Option<(Entity, Transform)>,
    /// Asset selected in the Asset Browser panel, by `Guid`. Drives the Inspector's asset view.
    /// Held on the overlay (not egui temp state) so the render system can resolve the asset's data
    /// snapshot before the egui frame runs.
    pub(crate) selected_asset: Option<kooch_core::Guid>,
    /// Which build preset the Build panel has selected (#758).
    pub(crate) build_selection: Option<kooch_core::Guid>,
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

/// A component's reflected field values, or why they are not here.
pub(crate) enum ReflectedFields {
    /// Read from the component.
    Values(Vec<(String, ReflectValue)>),
    /// The type is not registered for reflection. There is nothing to
    /// read and there never will be.
    Unreflected,
    /// Not read: nothing on screen needed this entity's values.
    NotGathered,
}

impl ReflectedFields {
    /// The values, if they were read.
    pub(crate) fn values(&self) -> Option<&Vec<(String, ReflectValue)>> {
        match self {
            Self::Values(values) => Some(values),
            Self::Unreflected | Self::NotGathered => None,
        }
    }

    /// Whether the type carries reflection at all — true even when the
    /// values were skipped, because the schema does not depend on
    /// whether anyone asked for them this frame.
    pub(crate) fn is_reflectable(&self) -> bool {
        !matches!(self, Self::Unreflected)
    }
}

/// Display data for a single component on an entity.
pub(crate) struct ComponentDisplayInfo {
    /// Local type handle, used for reflection and egui id salts. Absent
    /// on a remote client that has no Rust type for this component.
    pub(crate) type_id: TypeId,
    /// Portable identity, carried by any action this component emits.
    pub(crate) component: ComponentId,
    /// The type's name without its module path.
    pub(crate) short_name: std::borrow::Cow<'static, str>,
    pub(crate) fields: ReflectedFields,
    /// Static field metadata parallel to `fields`. Used to pick widget
    /// kinds (e.g. dropdown for `choices`) without re-querying the
    /// ComponentRegistry during the UI pass.
    pub(crate) field_metas: Option<&'static [FieldMeta]>,
    pub(crate) visibility: InspectorVisibility,
}

/// One open scene, as the World panel needs to show it.
#[derive(Debug, Clone)]
pub(crate) struct SceneDisplayInfo {
    pub(crate) id: kooch_core::Guid,
    /// File stem, or "Untitled" for a scene never saved.
    pub(crate) name: String,
    /// Where it came from, or `None` for one never saved.
    pub(crate) path: Option<std::path::PathBuf>,
    pub(crate) dirty: bool,
    pub(crate) active: bool,
}

pub(crate) struct EntityDisplayInfo {
    /// Whether this entity belongs to a prefab instance.
    pub(crate) is_prefab_instance: bool,
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
    pub(crate) scene: Option<kooch_core::Guid>,
    /// Parent's world-space rotation from `GlobalTransform`, if the entity has a parent and that
    /// parent has a `GlobalTransform`. Used to convert World-space edits back to the local rotation
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
