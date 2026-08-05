//! The Game view — the scene through the *gameplay* camera, rendered
//! beside the View panel rather than instead of it (#592).
//!
//! # Why it is a second view and not a second stage
//!
//! It shares the stage's mesh pool, scene instances and cull pipelines,
//! and owns only what depends on where its camera is: attachments, Hi-Z
//! occlusion state and cull buffers. That is [`ViewId`]. A second
//! `MeshletRenderStage` would recompile nine compute pipelines and
//! duplicate the pool for one extra camera.
//!
//! # Which camera
//!
//! The highest-priority active camera that is **not** the editor's.
//! Filtering by `Without<EditorCamera>` rather than by priority: the
//! editor camera ships at priority 1000 to outrank user cameras in the
//! View panel, and reusing that number as the identity test would break
//! the moment a game authored a camera at 1000 for its own reasons.
//!
//! # No gizmos
//!
//! Selection outlines, grids and handles are authoring aids. The Game
//! panel answers "what does the player see", and a gizmo in it is a lie.

use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_core::time::Time;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::query::{Query, filter::Without};
use kooch_render::SkyRenderPass;
use kooch_render::meshlet::{MeshletBlit, MeshletRenderStage, ViewId};

use crate::editor_camera::markers::EditorCamera;
use crate::viewport::target::ViewportTarget;

/// The Game panel's offscreen target and its handle into the stage.
pub(crate) struct GameView {
    pub target: ViewportTarget,
    /// This view's slot in the [`MeshletRenderStage`]. Not the primary —
    /// that one belongs to the View panel.
    pub view_id: ViewId,
    /// Whether the last frame found a gameplay camera. Drives the
    /// panel's placeholder text, so an empty Game panel says *why* it is
    /// empty instead of showing black.
    pub has_camera: bool,
    /// Whether the Game panel is the focused tab.
    ///
    /// Input only reaches the project while this is true. Written by the
    /// UI, read by `remote_input` in an earlier stage of the next frame —
    /// one frame stale, which is the same frame of latency remote Play
    /// already costs and is invisible at the scale of "did I click the
    /// panel before pressing a key".
    pub focused: bool,
}

impl GameView {
    pub fn new(
        device: &wgpu::Device,
        egui_renderer: &mut egui_wgpu::Renderer,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        stage: &mut MeshletRenderStage,
    ) -> Self {
        Self {
            target: ViewportTarget::new(device, egui_renderer, format, size),
            view_id: stage.create_view(device, size),
            has_camera: false,
            focused: false,
        }
    }
}

/// Renders the gameplay camera into `game.target`.
///
/// Returns `false` and leaves the target untouched when no gameplay
/// camera exists — the panel draws its placeholder rather than a stale
/// frame dressed up as a live one.
pub(crate) fn render_game_view(
    gpu: &GpuContext,
    sky_pass: &mut SkyRenderPass,
    game: &mut GameView,
    stage: &mut MeshletRenderStage,
    blit: &MeshletBlit,
    resources: &mut Resources,
) -> bool {
    let Some((view_proj, cam_pos)) = gameplay_camera_matrices(resources, game.target.aspect())
    else {
        game.has_camera = false;
        return false;
    };
    game.has_camera = true;

    // Per view: dragging this panel's divider must not reallocate the
    // View panel's attachments.
    stage.resize_view(game.view_id, gpu.device(), game.target.size());

    // The stage submits its own command buffer (cull + raster +
    // deferred) before the encoder below reads its colour view.
    let stats = stage.render_with_assets(
        game.view_id,
        gpu.device(),
        gpu.queue(),
        resources,
        view_proj,
        cam_pos,
    );

    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("game_view_encoder"),
        });

    let sky_drawn = if let Some(active_sky) = SkyRenderPass::active_sky(resources) {
        let time_secs = resources
            .get::<Time>()
            .map(|t| t.elapsed_secs())
            .unwrap_or(0.0);
        sky_pass.render(
            gpu.queue(),
            &mut encoder,
            game.target.view(),
            game.target.depth_view(),
            resources,
            game.target.aspect(),
            active_sky,
            time_secs,
        )
    } else {
        false
    };
    if !sky_drawn {
        super::render::clear_to_black(&mut encoder, game.target.view(), game.target.depth_view());
    }

    // Same per-frame truth the View panel uses: `instances_uploaded > 0`
    // iff the pipeline ran a real dispatch this frame. Gating on the
    // pool's registered count instead would keep blitting a colour view
    // the stage does not clear when it skips, leaving last frame's ghost
    // over the sky.
    if stats.instances_uploaded > 0
        && let Some(color) = stage.view_color_view(game.view_id)
    {
        blit.blit(gpu.device(), &mut encoder, color, game.target.view());
    }

    gpu.queue().submit(Some(encoder.finish()));
    true
}

/// Highest-priority active camera that is not the editor's.
fn gameplay_camera_matrices(
    resources: &Resources,
    aspect: f32,
) -> Option<(glam::Mat4, glam::Vec3)> {
    let query =
        Query::<(&PerspectiveCamera, &GlobalTransform), Without<EditorCamera>>::new(resources);
    let mut best: Option<(i32, glam::Mat4, glam::Vec3)> = None;
    query.for_each(|(cam, gt)| {
        if !cam.active {
            return;
        }
        if let Some((p, _, _)) = best
            && cam.priority <= p
        {
            return;
        }
        let world = gt.matrix;
        let view = world.inverse();
        let fov_y_rad = cam.fov.to_radians().max(1.0_f32.to_radians());
        let proj = kooch_render::perspective_rh_reverse_z(
            fov_y_rad,
            aspect.max(0.01),
            cam.near.max(0.001),
            cam.far.max(cam.near + 0.001),
        );
        best = Some((cam.priority, proj * view, world.w_axis.truncate()));
    });
    best.map(|(_, vp, p)| (vp, p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kooch_ecs::allocator::EntityAllocator;
    use kooch_ecs::archetype_registry::ArchetypeRegistry;
    use kooch_ecs::component::ComponentRegistry;
    use kooch_ecs::query::AccessTracker;

    /// Two cameras: the editor's at the priority it really ships with,
    /// and a gameplay one below it. The View panel picks the editor's;
    /// this panel must not.
    fn world_with_both_cameras(editor_priority: i32, game_priority: i32) -> Resources {
        let mut r = Resources::new();
        let mut alloc = EntityAllocator::new();
        let editor_cam = alloc.spawn();
        let game_cam = alloc.spawn();
        r.insert(alloc);

        let mut registry = ComponentRegistry::new();
        registry.register_cpu_reflected::<PerspectiveCamera>();
        registry.register_cpu_reflected::<GlobalTransform>();
        registry.register_cpu::<EditorCamera>();

        let mut archetypes = ArchetypeRegistry::new();
        let editor_sig = [
            std::any::TypeId::of::<PerspectiveCamera>(),
            std::any::TypeId::of::<GlobalTransform>(),
            std::any::TypeId::of::<EditorCamera>(),
        ]
        .into_iter()
        .collect();
        let game_sig = [
            std::any::TypeId::of::<PerspectiveCamera>(),
            std::any::TypeId::of::<GlobalTransform>(),
        ]
        .into_iter()
        .collect();
        let editor_arch = archetypes.get_or_create(editor_sig);
        let game_arch = archetypes.get_or_create(game_sig);

        for (entity, priority, x) in [
            (editor_cam, editor_priority, 10.0),
            (game_cam, game_priority, -7.0),
        ] {
            registry
                .get_cpu_mut::<PerspectiveCamera>()
                .expect("registered")
                .insert(
                    entity,
                    PerspectiveCamera {
                        priority,
                        active: true,
                        ..Default::default()
                    },
                );
            // Distinct positions so the assert can tell which camera the
            // matrices came from.
            registry
                .get_cpu_mut::<GlobalTransform>()
                .expect("registered")
                .insert(
                    entity,
                    GlobalTransform {
                        matrix: glam::Mat4::from_translation(glam::Vec3::new(x, 0.0, 0.0)),
                    },
                );
        }
        registry
            .get_cpu_mut::<EditorCamera>()
            .expect("registered")
            .insert(editor_cam, EditorCamera);
        archetypes.register_entity(editor_cam, editor_arch);
        archetypes.register_entity(game_cam, game_arch);

        r.insert(registry);
        r.insert(archetypes);
        r.insert(AccessTracker::new());
        r
    }

    #[test]
    fn the_editor_camera_is_never_the_game_camera() {
        // 1000 is EDITOR_CAMERA_PRIORITY: it outranks everything in the
        // View panel by design, which is exactly why picking "highest
        // priority" here would show the authoring camera.
        let r = world_with_both_cameras(1000, 0);
        let (_, cam_pos) = gameplay_camera_matrices(&r, 1.0).expect("a gameplay camera exists");
        assert_eq!(cam_pos.x, -7.0, "picked the editor camera");
    }

    #[test]
    fn a_gameplay_camera_at_the_editors_priority_still_wins() {
        // The identity test is the marker, not the number. A game that
        // authors a camera at 1000 for its own reasons must not make the
        // Game panel show the editor's view.
        let r = world_with_both_cameras(1000, 1000);
        let (_, cam_pos) = gameplay_camera_matrices(&r, 1.0).expect("a gameplay camera exists");
        assert_eq!(cam_pos.x, -7.0);
    }

    #[test]
    fn no_gameplay_camera_reports_none() {
        // Not "black": the panel needs to distinguish "nothing to show"
        // from "the game renders black", and says which component to add.
        let mut r = world_with_both_cameras(1000, 0);
        {
            let registry = r.get_mut::<ComponentRegistry>().expect("registered");
            let cams = registry
                .get_cpu_mut::<PerspectiveCamera>()
                .expect("registered");
            for (_, cam) in cams.iter_mut() {
                cam.active = false;
            }
        }
        assert!(gameplay_camera_matrices(&r, 1.0).is_none());
    }
}
