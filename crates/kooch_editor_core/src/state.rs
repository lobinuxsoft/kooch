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
///
/// `Serialize`/`Deserialize` enable persisting the dock layout between
/// editor sessions via [`crate::layout`].
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
    Console,
    /// Making a shipped game out of the project (#758).
    Build,
    /// Where the frame actually goes (#785).
    ///
    /// 🔴 The variant exists whether or not the `profiling` feature is
    /// compiled in, and the panel says so when it is not. A variant
    /// behind `#[cfg]` would make the serialised dock layout mean
    /// different things in two builds of the same editor — open the
    /// layout in the other one and deserialisation fails on a tab that
    /// does not exist, taking the user's whole arrangement with it.
    Profiler,
    /// The performance metrics as a REAL dock tab (#942-class ask from
    /// the user): the overlay sidebar drew translucent over the game
    /// view and could not be read. Sections pin out into floating
    /// windows from here; the overlay stays available behind its
    /// chevron but defaults hidden.
    Performance,
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

/// The parsed map rather than a guid: the panel edits it, and going back
/// to the asset server for every frame's draw would mean the edited copy
/// and the loaded one are two values of the same thing — the shape behind
/// every prefab bug in #611.
#[derive(Debug, Clone)]
pub(crate) struct OpenInputMap {
    pub path: std::path::PathBuf,
    /// What is being edited.
    ///
    /// A standalone action is held as a **map of one**, so the panel
    /// draws bindings, composites and processors with the same code
    /// either way. Unity does exactly this internally for its singleton
    /// actions: *"we do create a map for them that contains just the
    /// singleton action"*. Only the save path and the map-level controls
    /// differ, which is what `kind` selects.
    pub kind: OpenInputKind,
    pub map: kooch_input::actions::ActionMap,
    /// Set when the panel should be brought to the front. Cleared by the
    /// dock once it has done so.
    pub focus_requested: bool,
    /// What the properties pane is editing.
    ///
    /// With the document rather than in the panel, so adding an action
    /// can select it — Unity goes further and puts the new one straight
    /// into rename, which is the difference between "there is a new
    /// action somewhere" and "here it is, name it".
    pub selected: Option<crate::panels::input_map::Selection>,
    /// Whether this diverges from what is on disk.
    ///
    /// Edits land here and nowhere else until saved — the same contract a
    /// prefab has (`DirtyPrefabs`). An editor that wrote the file on every
    /// keystroke would make undo mean "read the file back", and a crash
    /// mid-edit would leave a half-written binding on disk.
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
    EditorTab::Build,
    EditorTab::Profiler,
    EditorTab::Performance,
];

impl EditorTab {
    /// Returns the display label with icon.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::World => format!("{} World", crate::icons::GLOBE),
            // "Edit View" / "Game View", the user's naming: both are
            // real views of the same world, one through the authoring
            // camera and one through the gameplay camera. NOT "World
            // View" — the entity-hierarchy panel is already called
            // World, and two near-homonym tabs cost more than they
            // say. The VARIANTS stay `View`/`Game`: they are the names
            // serialized into saved dock layouts, and renaming a
            // serialized name breaks data silently.
            Self::View => format!("{} Edit View", crate::icons::EYE),
            Self::Game => format!("{} Game View", crate::icons::GAME_CONTROLLER),
            Self::Inspector => format!("{} Inspector", crate::icons::SLIDERS),
            Self::Archetypes => format!("{} Archetypes", crate::icons::TREE_STRUCTURE),
            Self::Components => format!("{} Components", crate::icons::LIST_BULLETS),
            Self::AssetBrowser => format!("{} Assets", crate::icons::FOLDER_OPEN),
            Self::InputMap => format!("{} Input Map", crate::icons::SLIDERS),
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

/// Creates the default 3-panel dock layout: World | View + Game |
/// Inspector + Performance.
/// The performance metrics are a dock tab beside the Inspector; the
/// in-viewport overlay still exists behind its chevron for whoever
/// wants numbers over the picture, but defaults hidden — drawn over
/// the game it could not be read.
///
/// Game sits as a *sibling tab* of View rather than a split: the two
/// answer the same question from different cameras, so the common
/// gesture is flipping between them, not watching both. Unity, Unreal
/// and Godot all default this way, and anyone who wants them side by
/// side drags the tab out. View is listed first, so it is the one
/// showing when the editor opens.
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
pub(crate) type EulerCacheKey = (
    Entity,
    kooch_ecs::component::ComponentId,
    String,
    RotationDisplayMode,
);

/// Editor overlay state, stored as a resource.
///
/// Holds the egui context, winit integration state, wgpu renderer,
/// dock layout, and UI state (entity selection).
pub struct EditorOverlay {
    pub(crate) ctx: egui::Context,
    pub(crate) winit_state: SharedWinitState,
    pub(crate) renderer: egui_wgpu::Renderer,
    pub(crate) dock_state: DockState<EditorTab>,
    /// Which panel the keyboard belongs to.
    ///
    /// Session state rather than persisted: on a fresh start nothing is
    /// focused, so no panel answers the arrows until the user has said
    /// which one they mean (#661).
    pub(crate) focused_tab: Option<EditorTab>,
    /// The Asset Browser's keyboard cursor, and the rows the renderer drew
    /// last frame for it to walk.
    pub(crate) asset_nav: crate::panels::asset_browser::AssetNav,
    /// The Inspector's cursor over component sections.
    pub(crate) inspector_nav: crate::panels::inspector::InspectorNav,
    pub(crate) selected_entities: Vec<Entity>,
    /// Entities whose gizmos draw whether or not they are selected.
    ///
    /// # Why per entity and not per component type
    ///
    /// Switching a whole kind on answers "show me every gravity field",
    /// which is a real question but a different one. "Keep an eye on
    /// *this* camera while I work on it" is the common one, and doing it
    /// by type floods the viewport with every other camera to answer it.
    ///
    /// # Why the session and not a file
    ///
    /// A pin is a working gesture, not a property of the level, so it has
    /// no business in a scene that someone else opens. Persisting it
    /// per user would need a stable identity across restarts, and
    /// `PersistentId` counts from zero in every project — the same guid
    /// means a different entity in the next one.
    pub(crate) pinned_gizmos: std::collections::HashSet<Entity>,
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
///
/// Reading them costs a `String` and a `Vec` per field, per component,
/// per entity — 5.26 of the frame's 5.45 ms of gather on a 610-entity
/// scene (#691), for values only the Inspector reads, of the one entity
/// it shows. So they are read for the selection and skipped for
/// everything else.
///
/// Three states rather than an `Option`, because "not read" and "this
/// type has no reflection" are different facts and only one of them
/// means the component cannot be edited. Collapsed into `None` they
/// would be indistinguishable, and a panel that read the fields of an
/// unselected entity would quietly render it as unreflectable — no
/// error, no log, just a component that looks like it lost its schema.
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
    ///
    /// Deliberately not `Option<&Vec>` by `From`: a caller that wants to
    /// treat "absent" as one case has to say so at the call site.
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
    ///
    /// Borrowed for anything the registry knows: `component_name` hands
    /// back a `&'static str` and owning a copy of it cost 2440 `String`
    /// allocations per frame on a 610-entity scene (#666). Owned only for
    /// a parked component, whose name arrived over the wire.
    pub(crate) short_name: std::borrow::Cow<'static, str>,
    pub(crate) fields: ReflectedFields,
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
    pub(crate) id: kooch_core::Guid,
    /// File stem, or "Untitled" for a scene never saved.
    pub(crate) name: String,
    /// Where it came from, or `None` for one never saved.
    ///
    /// Carried beside the name so "Save" can write to the file the scene
    /// came from without asking, and fall back to asking when there is no
    /// file yet. The stem alone cannot say where it lives.
    pub(crate) path: Option<std::path::PathBuf>,
    pub(crate) dirty: bool,
    pub(crate) active: bool,
}

pub(crate) struct EntityDisplayInfo {
    /// Whether this entity belongs to a prefab instance.
    ///
    /// Gathered rather than looked up in the panel: the World panel draws
    /// with `&mut Ui` and the components live in a world.
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
