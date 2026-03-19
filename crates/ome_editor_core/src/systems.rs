//! Editor startup and render systems.

use std::sync::Arc;

use egui_dock::{DockArea, TabViewer};
use egui_wgpu::ScreenDescriptor;

use ome_core::event::{AppExit, Events};
use ome_core::gpu::GpuContext;
use ome_core::raw_event::RawEventHandler;
use ome_core::resource::Resources;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::entity::Entity;

use crate::actions::{apply_actions, EditorAction};
use crate::launch_screen::{self, LaunchAction};
use crate::menu_bar::draw_menu_bar;
use crate::undo::UndoStack;
use crate::panels::archetypes::draw_archetypes_content;
use crate::panels::components::draw_components_content;
use crate::panels::inspector::draw_inspector_content;
use crate::panels::view::draw_view_content;
use crate::panels::world::draw_world_content;
use crate::play_state::PlayState;
use crate::project_state::ProjectState;
use crate::queries::{
    gather_archetype_data, gather_component_types, gather_entity_data, gather_reflected_types,
};
use crate::state::{
    ArchetypeDisplayInfo, ComponentTypeInfo, EditorOverlay, EditorTab, EguiEventHandler,
    EntityDisplayInfo, ReflectedTypeInfo,
};
use crate::style::{configure_fonts, configure_style};

// ---------------------------------------------------------------------------
// Tab viewer (egui_dock)
// ---------------------------------------------------------------------------

struct EditorTabViewer<'a> {
    entities: &'a [EntityDisplayInfo],
    archetypes: &'a [ArchetypeDisplayInfo],
    component_types: &'a [ComponentTypeInfo],
    selected: &'a mut Vec<Entity>,
    reflected_types: &'a [ReflectedTypeInfo],
    actions: &'a mut Vec<EditorAction>,
    entity_count: usize,
    archetype_count: usize,
    active_archetype_count: usize,
    last_clicked_index: &'a mut Option<usize>,
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
            EditorTab::View => draw_view_content(ui),
            EditorTab::Inspector => draw_inspector_content(
                ui,
                self.entities,
                self.selected,
                self.reflected_types,
                self.actions,
            ),
            EditorTab::Archetypes => draw_archetypes_content(ui, self.archetypes),
            EditorTab::Components => draw_components_content(ui, self.component_types),
        }
    }

    fn closeable(&mut self, _tab: &mut Self::Tab) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Startup system: creates the egui context, winit state, wgpu renderer,
/// and configures fonts and dock layout.
pub fn editor_startup_system(resources: &mut Resources) {
    let gpu = resources
        .get::<GpuContext>()
        .expect("GpuContext not found — add WindowPlugin before EditorPlugin");
    let window_handle = resources
        .get::<ome_window::WindowHandle>()
        .expect("WindowHandle not found — add WindowPlugin before EditorPlugin");
    let window = window_handle.window();

    let ctx = egui::Context::default();
    configure_fonts(&ctx);
    configure_style(&ctx);

    let winit_state = Arc::new(std::sync::Mutex::new(egui_winit::State::new(
        ctx.clone(),
        egui::ViewportId::ROOT,
        window.as_ref(),
        Some(window.scale_factor() as f32),
        None,
        None,
    )));

    let renderer = egui_wgpu::Renderer::new(gpu.device(), gpu.format(), None, 1, false);

    let overlay = EditorOverlay {
        ctx,
        winit_state: Arc::clone(&winit_state),
        renderer,
        dock_state: crate::state::default_dock_state(),
        selected_entities: Vec::new(),
        last_clicked_index: None,
    };

    let handler: Box<dyn RawEventHandler> = Box::new(EguiEventHandler { winit_state });
    resources.insert(overlay);
    resources.insert(handler);

    tracing::info!("Editor overlay initialized");
}

/// Render system: runs egui UI and renders the overlay to the surface.
pub fn editor_render_system(resources: &mut Resources) {
    // 0. Poll game process state.
    let is_playing = if let Some(play_state) = resources.get_mut::<PlayState>() {
        play_state.poll();
        for line in play_state.drain_output() {
            tracing::info!("[game] {line}");
        }
        play_state.is_playing()
    } else {
        false
    };

    // Poll launcher process (compile + run project binary).
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.poll_launcher();
    }

    // If the project binary was launched, close the launcher.
    let launched = resources
        .get::<ProjectState>()
        .and_then(|ps| ps.launcher_status())
        .is_some_and(|s| *s == crate::project_state::LauncherStatus::Launched);
    if launched {
        if let Some(events) = resources.get_mut::<Events<AppExit>>() {
            events.send(AppExit);
        }
        return;
    }

    // Check if a project is loaded to decide between launch screen and editor.
    let project_loaded = resources
        .get::<ProjectState>()
        .map_or(false, |ps| ps.is_project_loaded());

    // 1. Gather ECS data only when a project is loaded.
    let (entities, archetype_data, component_types, reflected_types);
    let (entity_count, archetype_count, active_archetype_count);
    if project_loaded {
        entities = gather_entity_data(resources);
        archetype_data = gather_archetype_data(resources);
        component_types = gather_component_types(resources);
        reflected_types = gather_reflected_types(resources);
        entity_count = entities.len();
        archetype_count = resources
            .get::<ArchetypeRegistry>()
            .map_or(0, |a| a.archetype_count());
        active_archetype_count = archetype_data.iter().filter(|a| a.entity_count > 0).count();
    } else {
        entities = Vec::new();
        archetype_data = Vec::new();
        component_types = Vec::new();
        reflected_types = Vec::new();
        entity_count = 0;
        archetype_count = 0;
        active_archetype_count = 0;
    }

    // 2. Clone window Arc.
    let window = resources
        .get::<ome_window::WindowHandle>()
        .expect("WindowHandle not found")
        .window()
        .clone();

    // 3. Remove GpuContext, EditorOverlay, ProjectState, and UndoStack to avoid borrow conflicts.
    let gpu = resources
        .remove::<GpuContext>()
        .expect("GpuContext not found");
    let mut overlay = resources
        .remove::<EditorOverlay>()
        .expect("EditorOverlay not found");
    let mut project_state = resources.remove::<ProjectState>();
    let mut undo_stack = resources
        .remove::<UndoStack>()
        .unwrap_or_else(UndoStack::new);

    // Capture undo/redo state for the menu bar (before any actions this frame).
    let can_undo = undo_stack.can_undo();
    let can_redo = undo_stack.can_redo();
    let undo_desc = undo_stack.undo_description().map(String::from);
    let redo_desc = undo_stack.redo_description().map(String::from);

    // 4. Take accumulated egui input from winit events.
    let raw_input = {
        let mut state = overlay.winit_state.lock().unwrap();
        state.take_egui_input(&window)
    };

    // 5. Run egui UI — launch screen or editor depending on project state.
    let mut selected = std::mem::take(&mut overlay.selected_entities);
    let mut last_clicked_index = overlay.last_clicked_index.take();
    let mut actions: Vec<EditorAction> = Vec::new();

    let full_output = overlay.ctx.run(raw_input, |ctx| {
        if project_loaded {
            // --- Normal editor UI ---
            draw_menu_bar(
                ctx,
                &mut overlay.dock_state,
                &mut actions,
                is_playing,
                can_undo,
                can_redo,
                undo_desc.as_deref(),
                redo_desc.as_deref(),
            );

            let mut tab_viewer = EditorTabViewer {
                entities: &entities,
                archetypes: &archetype_data,
                component_types: &component_types,
                selected: &mut selected,
                reflected_types: &reflected_types,
                actions: &mut actions,
                entity_count,
                archetype_count,
                active_archetype_count,
                last_clicked_index: &mut last_clicked_index,
            };

            DockArea::new(&mut overlay.dock_state)
                .style(egui_dock::Style::from_egui(ctx.style().as_ref()))
                .show(ctx, &mut tab_viewer);
        } else if let Some(ref mut ps) = project_state {
            // --- Launch screen ---
            let launch_actions = launch_screen::draw_launch_screen(ctx, ps);
            for la in launch_actions {
                match la {
                    LaunchAction::OpenProject(path) => {
                        actions.push(EditorAction::OpenProject(path));
                    }
                    LaunchAction::CreateProject { name, parent_path } => {
                        actions.push(EditorAction::CreateProject { name, parent_path });
                    }
                    LaunchAction::RemoveRecent(path) => {
                        actions.push(EditorAction::RemoveRecent(path));
                    }
                    LaunchAction::LaunchProject(path) => {
                        actions.push(EditorAction::LaunchProject(path));
                    }
                    LaunchAction::CancelLaunch => {
                        actions.push(EditorAction::CancelLaunch);
                    }
                }
            }
        }
    });

    overlay.selected_entities = selected;
    overlay.last_clicked_index = last_clicked_index;

    // 6. Handle platform output (cursor icon, clipboard, etc.).
    {
        let mut state = overlay.winit_state.lock().unwrap();
        state.handle_platform_output(&window, full_output.platform_output);
    }

    // 7. Tessellate.
    let pixels_per_point = overlay.ctx.pixels_per_point();
    let clipped_primitives = overlay.ctx.tessellate(full_output.shapes, pixels_per_point);

    // 8. Render to surface.
    let (width, height) = gpu.size();
    let screen_descriptor = ScreenDescriptor {
        size_in_pixels: [width, height],
        pixels_per_point,
    };

    for (id, image_delta) in &full_output.textures_delta.set {
        overlay
            .renderer
            .update_texture(gpu.device(), gpu.queue(), *id, image_delta);
    }

    let mut encoder =
        gpu.device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("egui_encoder"),
            });

    let extra_buffers = overlay.renderer.update_buffers(
        gpu.device(),
        gpu.queue(),
        &mut encoder,
        &clipped_primitives,
        &screen_descriptor,
    );

    let output = match gpu.surface().get_current_texture() {
        Ok(tex) => tex,
        Err(e) => {
            tracing::warn!("Failed to acquire surface texture: {e}");
            resources.insert(gpu);
            resources.insert(overlay);
            resources.insert(undo_stack);
            if let Some(ps) = project_state {
                resources.insert(ps);
            }
            return;
        }
    };
    let view = output
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

    {
        let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.1,
                        g: 0.1,
                        b: 0.1,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        let mut render_pass = render_pass.forget_lifetime();
        overlay
            .renderer
            .render(&mut render_pass, &clipped_primitives, &screen_descriptor);
    }

    let mut buffers = extra_buffers;
    buffers.push(encoder.finish());
    gpu.queue().submit(buffers);
    output.present();

    for id in &full_output.textures_delta.free {
        overlay.renderer.free_texture(id);
    }

    // 9. Restore resources (except UndoStack — needed for apply_actions).
    resources.insert(gpu);
    resources.insert(overlay);
    if let Some(ps) = project_state {
        resources.insert(ps);
    }

    // 10. Apply deferred editor actions.
    if !actions.is_empty() {
        let has_open_scene = actions
            .iter()
            .any(|a| matches!(a, EditorAction::OpenScene));

        apply_actions(resources, &actions, &mut undo_stack);

        // Clear editor selection when a new scene is loaded.
        if has_open_scene {
            if let Some(overlay) = resources.get_mut::<EditorOverlay>() {
                overlay.selected_entities.clear();
                overlay.last_clicked_index = None;
            }
        }

        // GC empty archetypes after structural changes.
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.gc_empty_archetypes();
        }
    }

    // Restore UndoStack.
    resources.insert(undo_stack);
}
