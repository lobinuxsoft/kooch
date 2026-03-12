//! Editor overlay state and systems.
//!
//! Contains the [`EditorOverlay`] resource (egui context + renderer),
//! winit event forwarding, dockable panel layout, and the render system
//! that draws the overlay.

use std::any::{Any, TypeId};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
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
use ome_ecs::reflect::ReflectValue;

use crate::icons;

/// Shared egui-winit state for event forwarding between the
/// window event handler and the render system.
type SharedWinitState = Arc<Mutex<egui_winit::State>>;

// ---------------------------------------------------------------------------
// Dock tabs
// ---------------------------------------------------------------------------

/// Identifiers for each dockable editor tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum EditorTab {
    World,
    View,
    Inspector,
    Archetypes,
    Components,
}

/// All tab variants, used for the Window menu.
const ALL_TABS: &[EditorTab] = &[
    EditorTab::World,
    EditorTab::View,
    EditorTab::Inspector,
    EditorTab::Archetypes,
    EditorTab::Components,
];

impl EditorTab {
    /// Returns the display label with icon.
    fn label(&self) -> String {
        match self {
            Self::World => format!("{} World", icons::GLOBE),
            Self::View => format!("{} View", icons::EYE),
            Self::Inspector => format!("{} Inspector", icons::SLIDERS),
            Self::Archetypes => format!("{} Archetypes", icons::TREE_STRUCTURE),
            Self::Components => format!("{} Components", icons::LIST_BULLETS),
        }
    }
}

impl std::fmt::Display for EditorTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Creates the default 3-panel dock layout: World | View | Inspector.
fn default_dock_state() -> DockState<EditorTab> {
    let mut state = DockState::new(vec![EditorTab::View]);

    let surface = state.main_surface_mut();
    surface.split_left(NodeIndex::root(), 0.2, vec![EditorTab::World]);

    let surface = state.main_surface_mut();
    surface.split_right(NodeIndex::root(), 0.7, vec![EditorTab::Inspector]);

    state
}

/// Returns `true` if the given tab exists anywhere in the dock state.
fn dock_has_tab(dock_state: &DockState<EditorTab>, tab: &EditorTab) -> bool {
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
    ctx: egui::Context,
    winit_state: SharedWinitState,
    renderer: egui_wgpu::Renderer,
    dock_state: DockState<EditorTab>,
    selected_entities: Vec<Entity>,
    /// Anchor index for Shift+Click range selection in the World panel.
    last_clicked_index: Option<usize>,
}

/// Forwards raw winit events to egui for input processing.
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
// Display data (gathered before egui frame)
// ---------------------------------------------------------------------------

/// Display data for a single component on an entity.
struct ComponentDisplayInfo {
    type_id: TypeId,
    short_name: String,
    fields: Option<Vec<(String, ReflectValue)>>,
}

struct EntityDisplayInfo {
    entity: Entity,
    components: Vec<ComponentDisplayInfo>,
}

fn gather_entity_data(resources: &Resources) -> Vec<EntityDisplayInfo> {
    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return Vec::new();
    };
    let components = resources.get::<ComponentRegistry>();

    let mut entities = Vec::new();
    for archetype in archetypes.iter_matching(&[]) {
        for &entity in archetype.entities() {
            let comps: Vec<ComponentDisplayInfo> = archetype
                .components()
                .iter()
                .filter_map(|tid| {
                    let registry = components.as_ref()?;
                    let full_name = registry.component_name(tid)?;
                    let short_name = full_name
                        .rsplit("::")
                        .next()
                        .unwrap_or(full_name)
                        .to_owned();
                    let fields = registry.reflect_get_fields(tid, entity);
                    Some(ComponentDisplayInfo {
                        type_id: *tid,
                        short_name,
                        fields,
                    })
                })
                .collect();
            entities.push(EntityDisplayInfo {
                entity,
                components: comps,
            });
        }
    }
    entities.sort_by_key(|e| e.entity.index());
    entities
}

/// Display data for a single archetype.
struct ArchetypeDisplayInfo {
    id_short: String,
    entity_count: usize,
    component_names: Vec<String>,
}

fn gather_archetype_data(resources: &Resources) -> Vec<ArchetypeDisplayInfo> {
    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return Vec::new();
    };
    let components = resources.get::<ComponentRegistry>();

    let mut result = Vec::new();
    for archetype in archetypes.iter_matching(&[]) {
        let comp_names: Vec<String> = archetype
            .components()
            .iter()
            .map(|tid| {
                components
                    .as_ref()
                    .and_then(|r| r.component_name(tid))
                    .map(|name| name.rsplit("::").next().unwrap_or(name).to_owned())
                    .unwrap_or_else(|| format!("{:?}", tid))
            })
            .collect();

        result.push(ArchetypeDisplayInfo {
            id_short: format!("{:?}", archetype.id()),
            entity_count: archetype.len(),
            component_names: comp_names,
        });
    }
    result.sort_by(|a, b| b.entity_count.cmp(&a.entity_count));
    result
}

/// Display data for a registered component type.
struct ComponentTypeInfo {
    #[allow(dead_code)]
    type_id: TypeId,
    short_name: String,
    has_reflection: bool,
}

fn gather_component_types(resources: &Resources) -> Vec<ComponentTypeInfo> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let mut types: Vec<ComponentTypeInfo> = registry
        .all_type_names()
        .into_iter()
        .map(|(tid, name)| {
            let short = name.rsplit("::").next().unwrap_or(name).to_owned();
            ComponentTypeInfo {
                type_id: tid,
                short_name: short,
                has_reflection: registry.has_reflector(&tid),
            }
        })
        .collect();
    types.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    types
}

/// Available reflected component types for "Add Component".
struct ReflectedTypeInfo {
    type_id: TypeId,
    short_name: String,
}

fn gather_reflected_types(resources: &Resources) -> Vec<ReflectedTypeInfo> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let mut types: Vec<ReflectedTypeInfo> = registry
        .reflected_type_names()
        .into_iter()
        .map(|(tid, name)| {
            let short = name.rsplit("::").next().unwrap_or(name).to_owned();
            ReflectedTypeInfo {
                type_id: tid,
                short_name: short,
            }
        })
        .collect();
    types.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    types
}

// ---------------------------------------------------------------------------
// Editor actions (collected during UI, applied after render)
// ---------------------------------------------------------------------------

enum EditorAction {
    Spawn,
    Despawn(Entity),
    SetField {
        entity: Entity,
        type_id: TypeId,
        field: String,
        value: ReflectValue,
    },
    AddComponent {
        entity: Entity,
        type_id: TypeId,
    },
    RemoveComponent {
        entity: Entity,
        type_id: TypeId,
    },
}

fn apply_actions(resources: &mut Resources, actions: &[EditorAction]) {
    for action in actions {
        match action {
            EditorAction::Spawn => {
                let mut commands = match resources.remove::<Commands>() {
                    Some(c) => c,
                    None => return,
                };
                let entity = commands.spawn(resources).id();
                resources.insert(commands);

                // Auto-add Name and Transform defaults for editor-spawned entities.
                let default_components: Vec<TypeId> = resources
                    .get::<ComponentRegistry>()
                    .map(|reg| {
                        reg.reflected_type_names()
                            .into_iter()
                            .filter(|(_, name)| {
                                let short = name.rsplit("::").next().unwrap_or(name);
                                short == "Name" || short == "Transform"
                            })
                            .map(|(tid, _)| tid)
                            .collect()
                    })
                    .unwrap_or_default();

                for type_id in &default_components {
                    let mut inserted = false;
                    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                        inserted = registry.insert_default_reflected(type_id, entity);
                    }
                    if inserted {
                        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                            if let Some(current) = archetypes.entity_archetype(entity) {
                                let new_arch =
                                    archetypes.archetype_after_add_dynamic(current, *type_id);
                                archetypes.register_entity(entity, new_arch);
                            }
                        }
                    }
                }
            }
            EditorAction::Despawn(entity) => {
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
            EditorAction::SetField {
                entity,
                type_id,
                field,
                value,
            } => {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    if let Err(e) =
                        registry.reflect_set_field(type_id, *entity, field, value.clone())
                    {
                        tracing::warn!("failed to set field '{field}': {e}");
                    }
                }
            }
            EditorAction::AddComponent { entity, type_id } => {
                let mut inserted = false;
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    inserted = registry.insert_default_reflected(type_id, *entity);
                }
                if inserted {
                    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                        if let Some(current) = archetypes.entity_archetype(*entity) {
                            let new_arch =
                                archetypes.archetype_after_add_dynamic(current, *type_id);
                            archetypes.register_entity(*entity, new_arch);
                        }
                    }
                }
            }
            EditorAction::RemoveComponent { entity, type_id } => {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    registry.remove_component(*entity, type_id);
                }
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    if let Some(current) = archetypes.entity_archetype(*entity) {
                        let new_arch =
                            archetypes.archetype_after_remove_dynamic(current, *type_id);
                        archetypes.register_entity(*entity, new_arch);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Font configuration
// ---------------------------------------------------------------------------

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "firacode".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/FiraCode-Regular.ttf"
        ))),
    );

    fonts.font_data.insert(
        "phosphor".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Phosphor.ttf"
        ))),
    );

    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(0, "firacode".to_owned());
        family.push("phosphor".to_owned());
    }

    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        family.insert(0, "firacode".to_owned());
        family.push("phosphor".to_owned());
    }

    ctx.set_fonts(fonts);
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.window_rounding = egui::Rounding::same(6.0);
    style.visuals.menu_rounding = egui::Rounding::same(4.0);
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    ctx.set_style(style);
}

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

    let winit_state = Arc::new(Mutex::new(egui_winit::State::new(
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
        dock_state: default_dock_state(),
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
    // 1. Gather ECS data before borrowing overlay.
    let entities = gather_entity_data(resources);
    let archetype_data = gather_archetype_data(resources);
    let component_types = gather_component_types(resources);
    let reflected_types = gather_reflected_types(resources);
    let entity_count = entities.len();
    let archetype_count = resources
        .get::<ArchetypeRegistry>()
        .map_or(0, |a| a.archetype_count());
    let active_archetype_count = archetype_data.iter().filter(|a| a.entity_count > 0).count();

    // 2. Clone window Arc.
    let window = resources
        .get::<ome_window::WindowHandle>()
        .expect("WindowHandle not found")
        .window()
        .clone();

    // 3. Remove GpuContext and EditorOverlay to avoid borrow conflicts.
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

    // 5. Run egui UI with dock layout.
    let mut selected = std::mem::take(&mut overlay.selected_entities);
    let mut last_clicked_index = overlay.last_clicked_index.take();
    let mut actions: Vec<EditorAction> = Vec::new();

    let full_output = overlay.ctx.run(raw_input, |ctx| {
        draw_menu_bar(ctx, &mut overlay.dock_state);

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

    // 9. Restore resources.
    resources.insert(gpu);
    resources.insert(overlay);

    // 10. Apply deferred editor actions.
    if !actions.is_empty() {
        apply_actions(resources, &actions);

        // GC empty archetypes after structural changes.
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.gc_empty_archetypes();
        }
    }
}

// ---------------------------------------------------------------------------
// UI drawing functions
// ---------------------------------------------------------------------------

fn draw_menu_bar(ctx: &egui::Context, dock_state: &mut DockState<EditorTab>) {
    egui::TopBottomPanel::top("editor_menu").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("Window", |ui| {
                for &tab in ALL_TABS {
                    let is_open = dock_has_tab(dock_state, &tab);
                    if ui.selectable_label(is_open, tab.label()).clicked() {
                        if is_open {
                            dock_state.retain_tabs(|t| *t != tab);
                        } else {
                            dock_state.add_window(vec![tab]);
                        }
                        ui.close_menu();
                    }
                }
            });
        });
    });
}

/// Content of the "World" tab — entity hierarchy list with context menu.
fn draw_world_content(
    ui: &mut egui::Ui,
    entities: &[EntityDisplayInfo],
    selected: &mut Vec<Entity>,
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
    entity_count: usize,
    archetype_count: usize,
    active_archetype_count: usize,
    last_clicked_index: &mut Option<usize>,
) {
    ui.label(format!(
        "{} entities, {} archetypes ({} active)",
        entity_count, archetype_count, active_archetype_count,
    ));
    ui.separator();

    ui.horizontal(|ui| {
        if ui.button(format!("{} Spawn", icons::PLUS)).clicked() {
            actions.push(EditorAction::Spawn);
        }
        let can_despawn = !selected.is_empty();
        if ui
            .add_enabled(
                can_despawn,
                egui::Button::new(format!("{} Despawn", icons::TRASH)),
            )
            .clicked()
        {
            for entity in selected.drain(..) {
                actions.push(EditorAction::Despawn(entity));
            }
        }
    });
    ui.separator();

    // Delete/Suprimir: despawn selected entities.
    let kb_delete = ui.input(|i| i.key_pressed(egui::Key::Delete));
    if kb_delete && !selected.is_empty() {
        for entity in selected.drain(..) {
            actions.push(EditorAction::Despawn(entity));
        }
        *last_clicked_index = None;
    }

    // Keyboard navigation: Ctrl+A to select all.
    let kb_select_all = ui.input(|i| {
        i.modifiers.command && i.key_pressed(egui::Key::A)
    });
    if kb_select_all && !entities.is_empty() {
        selected.clear();
        selected.extend(entities.iter().map(|e| e.entity));
        *last_clicked_index = Some(entities.len() - 1);
    }

    // Keyboard navigation: Arrow Up/Down.
    let kb_up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
    let kb_down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
    let kb_shift = ui.input(|i| i.modifiers.shift);

    if (kb_up || kb_down) && !entities.is_empty() {
        let current_idx = last_clicked_index.unwrap_or(0);
        let new_idx = if kb_up {
            current_idx.saturating_sub(1)
        } else {
            (current_idx + 1).min(entities.len() - 1)
        };

        if kb_shift {
            // Extend selection to include the new index.
            let entity = entities[new_idx].entity;
            if !selected.contains(&entity) {
                selected.push(entity);
            }
        } else {
            // Move selection to the new index.
            selected.clear();
            selected.push(entities[new_idx].entity);
        }
        *last_clicked_index = Some(new_idx);
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (idx, info) in entities.iter().enumerate() {
            // If the entity has a Name component with a non-empty value,
            // display that instead of the raw index:generation.
            let display_name = info
                .components
                .iter()
                .find(|c| c.short_name == "Name")
                .and_then(|c| c.fields.as_ref())
                .and_then(|fields| {
                    fields.iter().find_map(|(name, val)| {
                        if name == "value" {
                            if let ReflectValue::String(s) = val {
                                if !s.is_empty() {
                                    return Some(s.clone());
                                }
                            }
                        }
                        None
                    })
                });

            let label = if let Some(name) = &display_name {
                format!(
                    "{} {}  [{}]",
                    icons::CUBE,
                    name,
                    info.components.len()
                )
            } else {
                format!(
                    "{} Entity {}:{}  [{}]",
                    icons::CUBE,
                    info.entity.index(),
                    info.entity.generation(),
                    info.components.len()
                )
            };
            let is_selected = selected.contains(&info.entity);
            let resp = ui.selectable_label(is_selected, &label);

            if resp.clicked() {
                let modifiers = ui.input(|i| i.modifiers);
                if modifiers.shift {
                    // Shift+Click: range selection from anchor to current.
                    let anchor = last_clicked_index.unwrap_or(0);
                    let range_start = anchor.min(idx);
                    let range_end = anchor.max(idx);
                    if !modifiers.ctrl && !modifiers.command {
                        selected.clear();
                    }
                    for i in range_start..=range_end {
                        let entity = entities[i].entity;
                        if !selected.contains(&entity) {
                            selected.push(entity);
                        }
                    }
                    // Don't update anchor on Shift+Click — keep the original.
                } else if modifiers.ctrl || modifiers.command {
                    // Ctrl+Click: toggle individual item.
                    if is_selected {
                        selected.retain(|e| *e != info.entity);
                    } else {
                        selected.push(info.entity);
                    }
                    *last_clicked_index = Some(idx);
                } else {
                    // Plain click: replace selection.
                    selected.clear();
                    selected.push(info.entity);
                    *last_clicked_index = Some(idx);
                }
            }

            // Right click: context menu.
            resp.context_menu(|ui| {
                // Ensure the right-clicked entity is selected.
                if !selected.contains(&info.entity) {
                    selected.clear();
                    selected.push(info.entity);
                }

                let count = selected.len();
                let label = if count == 1 {
                    format!("{} Despawn", icons::TRASH)
                } else {
                    format!("{} Despawn {} entities", icons::TRASH, count)
                };

                if ui.button(label).clicked() {
                    for entity in selected.drain(..) {
                        actions.push(EditorAction::Despawn(entity));
                    }
                    ui.close_menu();
                }

                // Add Component submenu (only for single entity).
                if selected.len() == 1 {
                    let entity = selected[0];
                    let existing: HashSet<TypeId> = entities
                        .iter()
                        .find(|e| e.entity == entity)
                        .map(|e| e.components.iter().map(|c| c.type_id).collect())
                        .unwrap_or_default();

                    let available: Vec<&ReflectedTypeInfo> = reflected_types
                        .iter()
                        .filter(|t| !existing.contains(&t.type_id))
                        .collect();

                    if !available.is_empty() {
                        ui.menu_button(
                            format!("{} Add Component", icons::PLUS),
                            |ui| {
                                for type_info in &available {
                                    if ui
                                        .selectable_label(false, &type_info.short_name)
                                        .clicked()
                                    {
                                        actions.push(EditorAction::AddComponent {
                                            entity,
                                            type_id: type_info.type_id,
                                        });
                                        ui.close_menu();
                                    }
                                }
                            },
                        );
                    }
                } else if selected.len() > 1 {
                    // Multi-select: add component to all selected.
                    ui.menu_button(
                        format!("{} Add Component to all", icons::PLUS),
                        |ui| {
                            for type_info in reflected_types {
                                if ui
                                    .selectable_label(false, &type_info.short_name)
                                    .clicked()
                                {
                                    for &entity in selected.iter() {
                                        actions.push(EditorAction::AddComponent {
                                            entity,
                                            type_id: type_info.type_id,
                                        });
                                    }
                                    ui.close_menu();
                                }
                            }
                        },
                    );

                    // Multi-select: remove shared component from all selected.
                    // Collect components present in ALL selected entities.
                    let selected_infos: Vec<&EntityDisplayInfo> = entities
                        .iter()
                        .filter(|e| selected.contains(&e.entity))
                        .collect();

                    if !selected_infos.is_empty() {
                        let mut shared: Vec<(TypeId, String)> = selected_infos[0]
                            .components
                            .iter()
                            .filter(|c| {
                                selected_infos[1..].iter().all(|info| {
                                    info.components.iter().any(|ic| ic.type_id == c.type_id)
                                })
                            })
                            .map(|c| (c.type_id, c.short_name.clone()))
                            .collect();
                        shared.sort_by(|a, b| a.1.cmp(&b.1));

                        if !shared.is_empty() {
                            ui.menu_button(
                                format!("{} Remove Component from all", icons::MINUS),
                                |ui| {
                                    for (type_id, name) in &shared {
                                        if ui.selectable_label(false, name).clicked() {
                                            for &entity in selected.iter() {
                                                actions.push(EditorAction::RemoveComponent {
                                                    entity,
                                                    type_id: *type_id,
                                                });
                                            }
                                            ui.close_menu();
                                        }
                                    }
                                },
                            );
                        }
                    }
                }
            });
        }
    });
}

/// Content of the "View" tab — placeholder viewport.
fn draw_view_content(ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.weak("Viewport — scene rendering will go here");
    });
}

/// Content of the "Inspector" tab — component details for selected entities.
fn draw_inspector_content(
    ui: &mut egui::Ui,
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
) {
    if selected.is_empty() {
        ui.weak("No entity selected");
        return;
    }

    if selected.len() > 1 {
        ui.label(format!("{} entities selected", selected.len()));
        ui.separator();
        for &entity in selected {
            ui.label(format!(
                "{} Entity {}:{}",
                icons::CUBE,
                entity.index(),
                entity.generation()
            ));
        }
        return;
    }

    // Single entity selected — show full inspector.
    let entity = selected[0];
    let Some(info) = entities.iter().find(|e| e.entity == entity) else {
        ui.weak("Entity not found (despawned?)");
        return;
    };

    let entity_name = info
        .components
        .iter()
        .find(|c| c.short_name == "Name")
        .and_then(|c| c.fields.as_ref())
        .and_then(|fields| {
            fields.iter().find_map(|(name, val)| {
                if name == "value" {
                    if let ReflectValue::String(s) = val {
                        if !s.is_empty() {
                            return Some(s.clone());
                        }
                    }
                }
                None
            })
        });

    if let Some(name) = &entity_name {
        ui.label(format!(
            "{} {}  ({}:{})",
            icons::CUBE,
            name,
            entity.index(),
            entity.generation()
        ));
    } else {
        ui.label(format!(
            "{} Entity  index: {}  generation: {}",
            icons::CUBE,
            entity.index(),
            entity.generation()
        ));
    }
    ui.separator();

    // "Add Component" dropdown.
    let existing: HashSet<TypeId> = info.components.iter().map(|c| c.type_id).collect();
    let available: Vec<&ReflectedTypeInfo> = reflected_types
        .iter()
        .filter(|t| !existing.contains(&t.type_id))
        .collect();

    if !available.is_empty() {
        egui::ComboBox::from_label(format!("{} Add Component", icons::PLUS))
            .selected_text("Select...")
            .show_ui(ui, |ui| {
                for type_info in &available {
                    if ui.selectable_label(false, &type_info.short_name).clicked() {
                        actions.push(EditorAction::AddComponent {
                            entity,
                            type_id: type_info.type_id,
                        });
                    }
                }
            });
        ui.separator();
    }

    if info.components.is_empty() {
        ui.weak("(no components)");
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for comp in &info.components {
            let id = ui.make_persistent_id(format!(
                "comp_{}_{:?}",
                entity.index(),
                comp.type_id
            ));
            egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                true,
            )
            .show_header(ui, |ui| {
                ui.strong(format!("{} {}", icons::PUZZLE_PIECE, &comp.short_name));
                if ui
                    .small_button(icons::X)
                    .on_hover_text("Remove component")
                    .clicked()
                {
                    actions.push(EditorAction::RemoveComponent {
                        entity,
                        type_id: comp.type_id,
                    });
                }
            })
            .body(|ui| {
                if let Some(fields) = &comp.fields {
                    if fields.is_empty() {
                        ui.weak("(no fields)");
                    } else {
                        draw_reflected_fields(ui, entity, comp.type_id, fields, actions);
                    }
                } else {
                    ui.weak("(no reflection)");
                }
            });
        }
    });
}

/// Content of the "Archetypes" tab.
fn draw_archetypes_content(ui: &mut egui::Ui, archetypes: &[ArchetypeDisplayInfo]) {
    let active = archetypes.iter().filter(|a| a.entity_count > 0).count();
    ui.label(format!(
        "{} archetypes ({} active, {} empty)",
        archetypes.len(),
        active,
        archetypes.len() - active,
    ));
    ui.separator();

    if archetypes.is_empty() {
        ui.weak("(none)");
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (i, arch) in archetypes.iter().enumerate() {
            let header = if arch.component_names.is_empty() {
                format!(
                    "{} Empty  —  {} entities",
                    icons::STACK, arch.entity_count,
                )
            } else {
                format!(
                    "{} [{}]  —  {} entities",
                    icons::STACK,
                    arch.component_names.join(", "),
                    arch.entity_count,
                )
            };

            // Dim empty archetypes.
            let is_empty = arch.entity_count == 0;

            let id = ui.make_persistent_id(format!("arch_{}", i));
            egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                false,
            )
            .show_header(ui, |ui| {
                if is_empty {
                    ui.weak(header);
                } else {
                    ui.label(header);
                }
            })
            .body(|ui| {
                ui.label(format!("ID: {}", arch.id_short));
                ui.label(format!("Entities: {}", arch.entity_count));
                if arch.component_names.is_empty() {
                    ui.weak("No components (empty archetype)");
                } else {
                    ui.label("Components:");
                    for name in &arch.component_names {
                        ui.label(format!("  {} {}", icons::PUZZLE_PIECE, name));
                    }
                }
            });
        }
    });
}

/// Content of the "Components" tab — lists all registered component types.
fn draw_components_content(ui: &mut egui::Ui, component_types: &[ComponentTypeInfo]) {
    let reflected = component_types.iter().filter(|c| c.has_reflection).count();
    ui.label(format!(
        "{} component types ({} with reflection)",
        component_types.len(),
        reflected,
    ));
    ui.separator();

    if component_types.is_empty() {
        ui.weak("(none)");
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for comp in component_types {
            ui.horizontal(|ui| {
                ui.label(format!("{} {}", icons::PUZZLE_PIECE, &comp.short_name));
                if comp.has_reflection {
                    ui.weak("(reflected)");
                } else {
                    ui.weak("(opaque)");
                }
            });
        }
    });
}

/// Renders editable widgets for reflected component fields.
fn draw_reflected_fields(
    ui: &mut egui::Ui,
    entity: Entity,
    type_id: TypeId,
    fields: &[(String, ReflectValue)],
    actions: &mut Vec<EditorAction>,
) {
    egui::Grid::new(format!("fields_{:?}_{}", type_id, entity.index()))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, value) in fields {
                ui.label(name);
                if let Some(new_value) = draw_value_widget(ui, value) {
                    actions.push(EditorAction::SetField {
                        entity,
                        type_id,
                        field: name.clone(),
                        value: new_value,
                    });
                }
                ui.end_row();
            }
        });
}

/// Draws an editable widget for a single reflected value.
/// Returns `Some(new_value)` if the user modified it.
fn draw_value_widget(ui: &mut egui::Ui, value: &ReflectValue) -> Option<ReflectValue> {
    match value {
        ReflectValue::F32(v) => {
            let mut val = *v;
            let resp = ui.add(egui::DragValue::new(&mut val).speed(0.1));
            resp.changed().then_some(ReflectValue::F32(val))
        }
        ReflectValue::F64(v) => {
            let mut val = *v as f32;
            let resp = ui.add(egui::DragValue::new(&mut val).speed(0.1));
            resp.changed().then_some(ReflectValue::F64(val as f64))
        }
        ReflectValue::U8(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val).range(0..=u8::MAX as i64));
            resp.changed().then_some(ReflectValue::U8(val as u8))
        }
        ReflectValue::U16(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val).range(0..=u16::MAX as i64));
            resp.changed().then_some(ReflectValue::U16(val as u16))
        }
        ReflectValue::U32(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val).range(0..=u32::MAX as i64));
            resp.changed().then_some(ReflectValue::U32(val as u32))
        }
        ReflectValue::U64(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val));
            resp.changed()
                .then_some(ReflectValue::U64(val.max(0) as u64))
        }
        ReflectValue::I8(v) => {
            let mut val = *v as i64;
            let resp = ui.add(
                egui::DragValue::new(&mut val).range(i8::MIN as i64..=i8::MAX as i64),
            );
            resp.changed().then_some(ReflectValue::I8(val as i8))
        }
        ReflectValue::I16(v) => {
            let mut val = *v as i64;
            let resp = ui.add(
                egui::DragValue::new(&mut val).range(i16::MIN as i64..=i16::MAX as i64),
            );
            resp.changed().then_some(ReflectValue::I16(val as i16))
        }
        ReflectValue::I32(v) => {
            let mut val = *v;
            let resp = ui.add(egui::DragValue::new(&mut val));
            resp.changed().then_some(ReflectValue::I32(val))
        }
        ReflectValue::I64(v) => {
            let mut val = *v;
            let resp = ui.add(egui::DragValue::new(&mut val));
            resp.changed().then_some(ReflectValue::I64(val))
        }
        ReflectValue::Bool(v) => {
            let mut val = *v;
            let resp = ui.checkbox(&mut val, "");
            resp.changed().then_some(ReflectValue::Bool(val))
        }
        ReflectValue::String(v) => {
            let mut val = v.clone();
            let resp = ui.text_edit_singleline(&mut val);
            resp.changed().then_some(ReflectValue::String(val))
        }
        ReflectValue::Vec2(v) => {
            let mut x = v.x;
            let mut y = v.y;
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("x");
                changed |= ui.add(egui::DragValue::new(&mut x).speed(0.1)).changed();
                ui.label("y");
                changed |= ui.add(egui::DragValue::new(&mut y).speed(0.1)).changed();
            });
            changed.then_some(ReflectValue::Vec2(glam::Vec2::new(x, y)))
        }
        ReflectValue::Vec3(v) => {
            let mut x = v.x;
            let mut y = v.y;
            let mut z = v.z;
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("x");
                changed |= ui.add(egui::DragValue::new(&mut x).speed(0.1)).changed();
                ui.label("y");
                changed |= ui.add(egui::DragValue::new(&mut y).speed(0.1)).changed();
                ui.label("z");
                changed |= ui.add(egui::DragValue::new(&mut z).speed(0.1)).changed();
            });
            changed.then_some(ReflectValue::Vec3(glam::Vec3::new(x, y, z)))
        }
        ReflectValue::Vec4(v) => {
            let mut vals = [v.x, v.y, v.z, v.w];
            let labels = ["x", "y", "z", "w"];
            let mut changed = false;
            ui.horizontal(|ui| {
                for (i, label) in labels.iter().enumerate() {
                    ui.label(*label);
                    changed |= ui
                        .add(egui::DragValue::new(&mut vals[i]).speed(0.1))
                        .changed();
                }
            });
            changed.then_some(ReflectValue::Vec4(glam::Vec4::new(
                vals[0], vals[1], vals[2], vals[3],
            )))
        }
        ReflectValue::Quat(v) => {
            // Display as Euler angles (degrees) for intuitive editing.
            let (rx, ry, rz) = v.to_euler(glam::EulerRot::XYZ);
            let mut dx = rx.to_degrees() + 0.0; // eliminate -0.0
            let mut dy = ry.to_degrees() + 0.0;
            let mut dz = rz.to_degrees() + 0.0;
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("x");
                changed |= ui
                    .add(egui::DragValue::new(&mut dx).speed(0.5).suffix("°"))
                    .changed();
                ui.label("y");
                changed |= ui
                    .add(egui::DragValue::new(&mut dy).speed(0.5).suffix("°"))
                    .changed();
                ui.label("z");
                changed |= ui
                    .add(egui::DragValue::new(&mut dz).speed(0.5).suffix("°"))
                    .changed();
            });
            changed.then_some(ReflectValue::Quat(glam::Quat::from_euler(
                glam::EulerRot::XYZ,
                dx.to_radians(),
                dy.to_radians(),
                dz.to_radians(),
            )))
        }
        ReflectValue::Mat4(_) => {
            ui.label("[Mat4]");
            None
        }
    }
}
