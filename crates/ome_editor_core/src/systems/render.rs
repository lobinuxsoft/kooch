//! Editor render system — runs egui UI and presents overlay to the surface.

use egui_dock::DockArea;

use ome_core::event::{AppExit, Events};
use ome_core::gpu::GpuContext;
use ome_core::resource::Resources;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_render::{MeshPassRenderer, RayMarchRenderer, SkyRenderPass};

use crate::actions::{apply_actions, EditorAction};
use crate::editor_camera::EditorCameraController;
use crate::editor_camera::input::{
    ViewportInputDelta, apply_viewport_input, entity_world_position,
};
use crate::launch_screen::{self, LaunchAction};
use crate::menu_bar::draw_menu_bar;
use crate::play_state::PlayState;
use crate::project_state::{LauncherStatus, ProjectState};
use crate::queries::{
    gather_archetype_data, gather_component_types, gather_entity_data, gather_reflected_types,
};
use crate::state::{
    ArchetypeDisplayInfo, ComponentTypeInfo, EditorOverlay, EntityDisplayInfo, ReflectedTypeInfo,
};
use crate::systems::present::present_editor_frame;
use crate::systems::tab_viewer::EditorTabViewer;
use crate::undo::UndoStack;
use crate::viewport::{render_viewport, ViewportTarget};

/// ECS data gathered once per frame, before the egui context runs.
struct FrameDisplayData {
    entities: Vec<EntityDisplayInfo>,
    archetypes: Vec<ArchetypeDisplayInfo>,
    component_types: Vec<ComponentTypeInfo>,
    reflected_types: Vec<ReflectedTypeInfo>,
    entity_count: usize,
    archetype_count: usize,
    active_archetype_count: usize,
}

impl FrameDisplayData {
    fn empty() -> Self {
        Self {
            entities: Vec::new(),
            archetypes: Vec::new(),
            component_types: Vec::new(),
            reflected_types: Vec::new(),
            entity_count: 0,
            archetype_count: 0,
            active_archetype_count: 0,
        }
    }

    fn gather(resources: &Resources) -> Self {
        let entities = gather_entity_data(resources);
        let archetypes = gather_archetype_data(resources);
        let component_types = gather_component_types(resources);
        let reflected_types = gather_reflected_types(resources);
        let entity_count = entities.len();
        let archetype_count = resources
            .get::<ArchetypeRegistry>()
            .map_or(0, |a| a.archetype_count());
        let active_archetype_count =
            archetypes.iter().filter(|a| a.entity_count > 0).count();
        Self {
            entities,
            archetypes,
            component_types,
            reflected_types,
            entity_count,
            archetype_count,
            active_archetype_count,
        }
    }
}

struct UndoInfo {
    can_undo: bool,
    can_redo: bool,
    undo_desc: Option<String>,
    redo_desc: Option<String>,
    is_playing: bool,
}

/// Polls launcher state. Returns `true` when the render system should exit early
/// because the project binary has been launched (triggering AppExit).
fn poll_launcher(resources: &mut Resources) -> bool {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.poll_launcher();
    }

    let launched = resources
        .get::<ProjectState>()
        .and_then(|ps| ps.launcher_status())
        .is_some_and(|s| *s == LauncherStatus::Launched);

    if launched
        && let Some(events) = resources.get_mut::<Events<AppExit>>()
    {
        events.send(AppExit);
    }
    launched
}

/// Handles to the viewport resource consumed by the UI: a read-only
/// texture id for drawing, slots where the View panel writes the
/// desired backing texture size and the captured input delta for the
/// frame, and a read-only snapshot of the camera controller used for
/// sensitivity reads inside the egui closure.
struct ViewportUi<'a> {
    texture_id: egui::TextureId,
    request: &'a mut Option<(u32, u32)>,
    input: &'a mut Option<ViewportInputDelta>,
    controller: &'a EditorCameraController,
}

/// Runs the egui UI for one frame. Produces the tessellation input and the
/// editor actions queued by widgets.
fn run_editor_ui(
    overlay: &mut EditorOverlay,
    project_state: &mut Option<ProjectState>,
    raw_input: egui::RawInput,
    project_loaded: bool,
    data: &FrameDisplayData,
    undo: &UndoInfo,
    viewport: ViewportUi<'_>,
) -> (egui::FullOutput, Vec<EditorAction>) {
    let mut selected = std::mem::take(&mut overlay.selected_entities);
    let mut last_clicked_index = overlay.last_clicked_index.take();
    let mut actions: Vec<EditorAction> = Vec::new();
    let ViewportUi {
        texture_id,
        request,
        input,
        controller,
    } = viewport;

    let full_output = overlay.ctx.run(raw_input, |ctx| {
        if project_loaded {
            draw_menu_bar(
                ctx,
                &mut overlay.dock_state,
                &mut actions,
                undo.is_playing,
                undo.can_undo,
                undo.can_redo,
                undo.undo_desc.as_deref(),
                undo.redo_desc.as_deref(),
            );

            let mut tab_viewer = EditorTabViewer {
                entities: &data.entities,
                archetypes: &data.archetypes,
                component_types: &data.component_types,
                selected: &mut selected,
                reflected_types: &data.reflected_types,
                actions: &mut actions,
                entity_count: data.entity_count,
                archetype_count: data.archetype_count,
                active_archetype_count: data.active_archetype_count,
                last_clicked_index: &mut last_clicked_index,
                viewport_texture_id: texture_id,
                viewport_request: request,
                viewport_input: input,
                editor_camera_controller: controller,
                rotation_euler_cache: &mut overlay.rotation_euler_cache,
                rotation_display_mode: &mut overlay.rotation_display_mode,
            };

            DockArea::new(&mut overlay.dock_state)
                .style(egui_dock::Style::from_egui(ctx.style().as_ref()))
                .show(ctx, &mut tab_viewer);
        } else if let Some(ps) = project_state.as_mut() {
            let launch_actions = launch_screen::draw_launch_screen(ctx, ps);
            forward_launch_actions(launch_actions, &mut actions);
        }
    });

    overlay.selected_entities = selected;
    overlay.last_clicked_index = last_clicked_index;
    (full_output, actions)
}

fn forward_launch_actions(launch_actions: Vec<LaunchAction>, actions: &mut Vec<EditorAction>) {
    for la in launch_actions {
        match la {
            LaunchAction::OpenProject(path) => actions.push(EditorAction::OpenProject(path)),
            LaunchAction::CreateProject { name, parent_path } => {
                actions.push(EditorAction::CreateProject { name, parent_path })
            }
            LaunchAction::RemoveRecent(path) => actions.push(EditorAction::RemoveRecent(path)),
            LaunchAction::LaunchProject(path) => actions.push(EditorAction::LaunchProject(path)),
            LaunchAction::CancelLaunch => actions.push(EditorAction::CancelLaunch),
        }
    }
}

fn apply_deferred_actions(
    resources: &mut Resources,
    actions: &[EditorAction],
    undo_stack: &mut UndoStack,
) {
    if actions.is_empty() {
        return;
    }
    let has_open_scene = actions
        .iter()
        .any(|a| matches!(a, EditorAction::OpenScene));

    apply_actions(resources, actions, undo_stack);

    if has_open_scene
        && let Some(overlay) = resources.get_mut::<EditorOverlay>()
    {
        overlay.selected_entities.clear();
        overlay.last_clicked_index = None;
    }

    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        archetypes.gc_empty_archetypes();
    }
}

/// Render system: runs egui UI and renders the overlay to the surface.
pub(crate) fn editor_render_system(resources: &mut Resources) {
    let is_playing = if let Some(play_state) = resources.get_mut::<PlayState>() {
        play_state.poll();
        for line in play_state.drain_output() {
            tracing::info!("[game] {line}");
        }
        play_state.is_playing()
    } else {
        false
    };

    if poll_launcher(resources) {
        return;
    }

    let project_loaded = resources
        .get::<ProjectState>()
        .is_some_and(|ps| ps.is_project_loaded());

    let display_data = if project_loaded {
        FrameDisplayData::gather(resources)
    } else {
        FrameDisplayData::empty()
    };

    let window = resources
        .get::<ome_window::WindowHandle>()
        .expect("WindowHandle not found")
        .window()
        .clone();

    let gpu = resources
        .remove::<GpuContext>()
        .expect("GpuContext not found");
    let mut overlay = resources
        .remove::<EditorOverlay>()
        .expect("EditorOverlay not found");
    let mut viewport = resources
        .remove::<ViewportTarget>()
        .expect("ViewportTarget not found");
    let mut raymarch = resources
        .remove::<RayMarchRenderer>()
        .expect("RayMarchRenderer not found");
    let mut mesh_pass = resources
        .remove::<MeshPassRenderer>()
        .expect("MeshPassRenderer not found");
    let mut sky_pass = resources
        .remove::<SkyRenderPass>()
        .expect("SkyRenderPass not found");
    let mut project_state = resources.remove::<ProjectState>();
    let mut undo_stack = resources
        .remove::<UndoStack>()
        .unwrap_or_else(UndoStack::new);

    // Apply the previous frame's size request before the UI runs so the
    // texture id stays stable through the entire egui pass.
    viewport.resize_if_needed(gpu.device(), &mut overlay.renderer);

    let undo = UndoInfo {
        can_undo: undo_stack.can_undo(),
        can_redo: undo_stack.can_redo(),
        undo_desc: undo_stack.undo_description().map(String::from),
        redo_desc: undo_stack.redo_description().map(String::from),
        is_playing,
    };

    let raw_input = {
        let mut state = overlay.winit_state.lock().unwrap();
        state.take_egui_input(&window)
    };

    let mut viewport_request: Option<(u32, u32)> = None;
    let mut viewport_input: Option<ViewportInputDelta> = None;
    let controller_snapshot = resources
        .get::<EditorCameraController>()
        .cloned()
        .unwrap_or_default();
    let (full_output, actions) = run_editor_ui(
        &mut overlay,
        &mut project_state,
        raw_input,
        project_loaded,
        &display_data,
        &undo,
        ViewportUi {
            texture_id: viewport.texture_id(),
            request: &mut viewport_request,
            input: &mut viewport_input,
            controller: &controller_snapshot,
        },
    );

    if let Some(size) = viewport_request {
        viewport.request_size(size);
    }

    // Apply viewport input to the editor camera before the same frame's
    // render pass so the new pose is visible immediately. Focus-on-
    // selection uses the first selected entity's world position, if any.
    if let Some(delta) = viewport_input {
        let selection_world = overlay
            .selected_entities
            .first()
            .copied()
            .and_then(|e| entity_world_position(resources, e));
        apply_viewport_input(delta, resources, selection_world);
    }

    render_viewport(
        &gpu,
        &mut sky_pass,
        &mut raymarch,
        &mut mesh_pass,
        &viewport,
        resources,
        project_loaded,
    );

    let _presented = present_editor_frame(&gpu, &mut overlay, &window, full_output);

    resources.insert(gpu);
    resources.insert(overlay);
    resources.insert(viewport);
    resources.insert(raymarch);
    resources.insert(mesh_pass);
    resources.insert(sky_pass);
    if let Some(ps) = project_state {
        resources.insert(ps);
    }

    apply_deferred_actions(resources, &actions, &mut undo_stack);

    resources.insert(undo_stack);
}
