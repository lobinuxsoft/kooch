//! Editor overlay types and state.

use std::any::{Any, TypeId};
use std::sync::{Arc, Mutex};

use egui_dock::{DockState, NodeIndex};
use winit::event::WindowEvent;
use winit::window::Window;

use ome_core::raw_event::RawEventHandler;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::ReflectValue;

/// Shared egui-winit state for event forwarding between the
/// window event handler and the render system.
pub(crate) type SharedWinitState = Arc<Mutex<egui_winit::State>>;

// ---------------------------------------------------------------------------
// Dock tabs
// ---------------------------------------------------------------------------

/// Identifiers for each dockable editor tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EditorTab {
    World,
    View,
    Inspector,
    Archetypes,
    Components,
}

/// All tab variants, used for the Window menu.
pub(crate) const ALL_TABS: &[EditorTab] = &[
    EditorTab::World,
    EditorTab::View,
    EditorTab::Inspector,
    EditorTab::Archetypes,
    EditorTab::Components,
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
        }
    }
}

impl std::fmt::Display for EditorTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Creates the default 3-panel dock layout: World | View | Inspector.
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
    pub(crate) type_id: TypeId,
    pub(crate) short_name: String,
    pub(crate) fields: Option<Vec<(String, ReflectValue)>>,
}

pub(crate) struct EntityDisplayInfo {
    pub(crate) entity: Entity,
    pub(crate) components: Vec<ComponentDisplayInfo>,
}

/// Display data for a single archetype.
pub(crate) struct ArchetypeDisplayInfo {
    pub(crate) id_short: String,
    pub(crate) entity_count: usize,
    pub(crate) component_names: Vec<String>,
}

/// Display data for a registered component type.
pub(crate) struct ComponentTypeInfo {
    #[allow(dead_code)]
    pub(crate) type_id: TypeId,
    pub(crate) short_name: String,
    pub(crate) has_reflection: bool,
}

/// Available reflected component types for "Add Component".
pub(crate) struct ReflectedTypeInfo {
    pub(crate) type_id: TypeId,
    pub(crate) short_name: String,
}
