//! Editor overlay state and systems.
//!
//! Contains the [`EditorOverlay`] resource (egui context + renderer),
//! winit event forwarding, and the render system that draws the overlay.

use std::any::Any;
use std::sync::{Arc, Mutex};

use egui_wgpu::ScreenDescriptor;
use winit::event::WindowEvent;
use winit::window::Window;

use ome_core::gpu::GpuContext;
use ome_core::raw_event::RawEventHandler;
use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_window::WindowHandle;

/// Shared egui-winit state for event forwarding between the
/// window event handler and the render system.
type SharedWinitState = Arc<Mutex<egui_winit::State>>;

/// Editor overlay state, stored as a resource.
///
/// Holds the egui context, winit integration state, wgpu renderer,
/// and UI state (selection, panel visibility).
pub struct EditorOverlay {
    ctx: egui::Context,
    winit_state: SharedWinitState,
    renderer: egui_wgpu::Renderer,
    selected_entity: Option<Entity>,
    show_hierarchy: bool,
    show_inspector: bool,
}

/// Forwards raw winit events to egui for input processing.
///
/// Stored as `Box<dyn RawEventHandler>` in resources. Called by
/// `WinitApp::window_event` before the frame tick.
struct EguiEventHandler {
    winit_state: SharedWinitState,
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
// Entity display data (gathered before egui frame)
// ---------------------------------------------------------------------------

struct EntityDisplayInfo {
    entity: Entity,
    component_names: Vec<&'static str>,
}

fn gather_entity_data(resources: &Resources) -> Vec<EntityDisplayInfo> {
    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return Vec::new();
    };
    let components = resources.get::<ComponentRegistry>();

    let mut entities = Vec::new();
    for archetype in archetypes.iter_matching(&[]) {
        for &entity in archetype.entities() {
            let names: Vec<&'static str> = archetype
                .components()
                .iter()
                .filter_map(|tid| components.and_then(|c| c.component_name(tid)))
                .collect();
            entities.push(EntityDisplayInfo {
                entity,
                component_names: names,
            });
        }
    }
    entities.sort_by_key(|e| e.entity.index());
    entities
}

// ---------------------------------------------------------------------------
// Editor actions (collected during UI, applied after render)
// ---------------------------------------------------------------------------

enum EditorAction {
    Spawn,
    Despawn(Entity),
}

fn apply_actions(resources: &mut Resources, actions: &[EditorAction]) {
    for action in actions {
        match action {
            EditorAction::Spawn => {
                let mut commands = match resources.remove::<Commands>() {
                    Some(c) => c,
                    None => return,
                };
                commands.spawn(resources);
                // Builder drops here → commits to queue.
                // Will be applied next frame by commands_apply_system.
                resources.insert(commands);
            }
            EditorAction::Despawn(entity) => {
                // Immediate despawn via allocator + archetype cleanup.
                if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
                    alloc.despawn(*entity);
                }
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    archetypes.unregister_entity(*entity);
                }
                if let Some(components) = resources.get_mut::<ComponentRegistry>() {
                    components.remove_entity(*entity);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Startup system: creates the egui context, winit state, and wgpu renderer.
pub fn editor_startup_system(resources: &mut Resources) {
    let gpu = resources
        .get::<GpuContext>()
        .expect("GpuContext not found — add WindowPlugin before EditorPlugin");
    let window_handle = resources
        .get::<WindowHandle>()
        .expect("WindowHandle not found — add WindowPlugin before EditorPlugin");
    let window = window_handle.window();

    let ctx = egui::Context::default();
    let winit_state = Arc::new(Mutex::new(egui_winit::State::new(
        ctx.clone(),
        egui::ViewportId::ROOT,
        window.as_ref(),
        Some(window.scale_factor() as f32),
        None,
        None,
    )));

    let renderer = egui_wgpu::Renderer::new(
        gpu.device(),
        gpu.format(),
        None,
        1,
        false,
    );

    let overlay = EditorOverlay {
        ctx,
        winit_state: Arc::clone(&winit_state),
        renderer,
        selected_entity: None,
        show_hierarchy: true,
        show_inspector: true,
    };

    let handler: Box<dyn RawEventHandler> = Box::new(EguiEventHandler { winit_state });
    resources.insert(overlay);
    resources.insert(handler);

    tracing::info!("Editor overlay initialized");
}

/// Render system: runs egui UI and renders the overlay to the surface.
pub fn editor_render_system(resources: &mut Resources) {
    // 1. Gather ECS data before borrowing overlay.
    let entities = gather_entity_data(resources);
    let entity_count = entities.len();
    let archetype_count = resources
        .get::<ArchetypeRegistry>()
        .map_or(0, |a| a.archetype_count());

    // 2. Clone window Arc.
    let window = resources
        .get::<WindowHandle>()
        .expect("WindowHandle not found")
        .window()
        .clone();

    // 3. Remove GpuContext and EditorOverlay to avoid borrow conflicts
    //    (same pattern as Schedule::run_gpu_batch).
    let gpu = resources
        .remove::<GpuContext>()
        .expect("GpuContext not found");
    let mut overlay = resources
        .remove::<EditorOverlay>()
        .expect("EditorOverlay not found");

    // 4. Take accumulated egui input from winit events.
    let raw_input = {
        let mut state = overlay.winit_state.lock().unwrap();
        state.take_egui_input(&window)
    };

    // 5. Run egui UI.
    let mut selected = overlay.selected_entity;
    let mut show_hierarchy = overlay.show_hierarchy;
    let mut show_inspector = overlay.show_inspector;
    let mut actions: Vec<EditorAction> = Vec::new();

    let full_output = overlay.ctx.run(raw_input, |ctx| {
        draw_menu_bar(ctx, &mut show_hierarchy, &mut show_inspector);

        if show_hierarchy {
            draw_hierarchy(
                ctx,
                &entities,
                &mut selected,
                &mut actions,
                entity_count,
                archetype_count,
            );
        }

        if show_inspector {
            draw_inspector(ctx, &entities, selected);
        }
    });

    overlay.selected_entity = selected;
    overlay.show_hierarchy = show_hierarchy;
    overlay.show_inspector = show_inspector;

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

        // forget_lifetime converts RenderPass<'encoder> → RenderPass<'static>
        // as required by egui-wgpu. Safe because the pass is dropped before
        // encoder.finish() below.
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

    // 9. Restore resources.
    resources.insert(gpu);
    resources.insert(overlay);

    // 10. Apply deferred editor actions (spawn/despawn).
    if !actions.is_empty() {
        apply_actions(resources, &actions);
    }
}

// ---------------------------------------------------------------------------
// UI drawing functions
// ---------------------------------------------------------------------------

fn draw_menu_bar(
    ctx: &egui::Context,
    show_hierarchy: &mut bool,
    show_inspector: &mut bool,
) {
    egui::TopBottomPanel::top("editor_menu").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("View", |ui| {
                ui.checkbox(show_hierarchy, "Hierarchy");
                ui.checkbox(show_inspector, "Inspector");
            });
        });
    });
}

fn draw_hierarchy(
    ctx: &egui::Context,
    entities: &[EntityDisplayInfo],
    selected: &mut Option<Entity>,
    actions: &mut Vec<EditorAction>,
    entity_count: usize,
    archetype_count: usize,
) {
    egui::SidePanel::left("hierarchy")
        .default_width(220.0)
        .show(ctx, |ui| {
            ui.heading("Entities");
            ui.label(format!(
                "{} entities, {} archetypes",
                entity_count, archetype_count
            ));
            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("Spawn").clicked() {
                    actions.push(EditorAction::Spawn);
                }
                let can_despawn = selected.is_some();
                if ui.add_enabled(can_despawn, egui::Button::new("Despawn")).clicked() {
                    if let Some(entity) = *selected {
                        actions.push(EditorAction::Despawn(entity));
                        *selected = None;
                    }
                }
            });
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                for info in entities {
                    let label = format!(
                        "Entity {}:{}  [{}]",
                        info.entity.index(),
                        info.entity.generation(),
                        info.component_names.len()
                    );
                    let is_selected = *selected == Some(info.entity);
                    if ui.selectable_label(is_selected, &label).clicked() {
                        *selected = if is_selected {
                            None
                        } else {
                            Some(info.entity)
                        };
                    }
                }
            });
        });
}

fn draw_inspector(
    ctx: &egui::Context,
    entities: &[EntityDisplayInfo],
    selected: Option<Entity>,
) {
    egui::SidePanel::right("inspector")
        .default_width(280.0)
        .show(ctx, |ui| {
            ui.heading("Inspector");
            ui.separator();

            let Some(entity) = selected else {
                ui.label("No entity selected");
                return;
            };

            let Some(info) = entities.iter().find(|e| e.entity == entity) else {
                ui.label("Entity not found (despawned?)");
                return;
            };

            ui.label(format!(
                "Entity  index: {}  generation: {}",
                entity.index(),
                entity.generation()
            ));
            ui.separator();

            ui.heading("Components");
            if info.component_names.is_empty() {
                ui.weak("(none)");
            } else {
                for name in &info.component_names {
                    let short = name.rsplit("::").next().unwrap_or(name);
                    ui.label(format!("  \u{2022} {}", short));
                }
            }
        });
}
