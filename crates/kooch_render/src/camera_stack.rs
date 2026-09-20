//! The cameras of one frame, in the order they compose (#1221).
//!
//! A frame has one base camera — the one that owns the image, its sky and its clear — and any
//! number of overlays drawn over it. Each camera is a view of its own, so an overlay carries its own
//! culling mask, its own lens and its own temporal history; what makes it an overlay is only that it
//! arrives after the base and keeps what it did not draw over.

use kooch_core::resource::Resources;
use kooch_ecs::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::query::Query;
use kooch_ecs::query::filter::QueryFilter;

use crate::meshlet::{MeshletRenderStage, ViewId};
use crate::view_camera::ViewCamera;

/// The base camera and its overlays, resolved once so two passes cannot disagree about either.
#[derive(Clone, Debug, Default)]
pub struct CameraStack {
    /// The highest-priority active camera that is not an overlay.
    pub base: Option<ViewCamera>,
    /// Lowest priority first: the order they are drawn in, so the last one is on top.
    pub overlays: Vec<(Entity, ViewCamera)>,
}

impl CameraStack {
    /// Reads the stack out of the ECS, skipping whatever `F` excludes — the editor excludes its own
    /// camera, a game excludes nothing.
    pub fn read<F: QueryFilter>(resources: &Resources) -> Self {
        let query = Query::<(&PerspectiveCamera, &GlobalTransform), F>::new(resources);
        let mut base: Option<(i32, u32, ViewCamera)> = None;
        let mut overlays: Vec<(i32, u32, Entity, ViewCamera)> = Vec::new();
        query.for_each_entity(|entity, (cam, transform)| {
            if !cam.active {
                return;
            }
            let view = ViewCamera::from_components(cam, transform);
            let order = (cam.priority, entity.index());
            match cam.overlay {
                true => overlays.push((order.0, order.1, entity, view)),
                // Priority first, index to break a tie: the same contract the editor and the game
                // runtime already picked their one camera by.
                false if base.is_none_or(|(p, i, _)| (order.0, order.1) > (p, i)) => {
                    base = Some((order.0, order.1, view));
                }
                false => {}
            }
        });
        overlays.sort_by_key(|(priority, index, _, _)| (*priority, *index));
        // 🔴 An overlay is defined against a base. With nothing under it there is nothing to keep
        // where it drew nothing, so it composes as the whole image — which is not what it asked for.
        if base.is_none() {
            overlays.clear();
        }
        Self {
            base: base.map(|(_, _, view)| view),
            overlays: overlays
                .into_iter()
                .map(|(_, _, entity, view)| (entity, view))
                .collect(),
        }
    }
}

/// The views the overlays render into, one per camera entity and kept across frames: a view is where
/// the temporal history lives, so an overlay that got a fresh one every frame would never converge.
#[derive(Default)]
pub struct StackViews {
    live: Vec<StackView>,
}

struct StackView {
    entity: Entity,
    view: ViewId,
    /// Whether the last frame really drew it. A view nobody rendered holds the frame before, and
    /// compositing that is last frame's ghost.
    drawn: bool,
}

impl StackViews {
    /// The view `entity` renders into, made at `size` the first time and resized after.
    pub fn view_for(
        &mut self,
        entity: Entity,
        stage: &mut MeshletRenderStage,
        device: &wgpu::Device,
        size: (u32, u32),
    ) -> ViewId {
        if let Some(live) = self.live.iter().find(|live| live.entity == entity) {
            let view = live.view;
            stage.resize_view(view, device, size);
            return view;
        }
        let view = stage.create_view(device, size);
        self.live.push(StackView {
            entity,
            view,
            drawn: false,
        });
        view
    }

    /// Records whether that view has this frame's image in it.
    pub fn mark(&mut self, entity: Entity, drawn: bool) {
        if let Some(live) = self.live.iter_mut().find(|live| live.entity == entity) {
            live.drawn = drawn;
        }
    }

    /// Frees the views of cameras the stack no longer holds: a deleted overlay camera keeping its
    /// attachments alive is a leak the size of a frame.
    pub fn retain(&mut self, stack: &CameraStack, stage: &mut MeshletRenderStage) {
        self.live.retain(|live| {
            let kept = stack
                .overlays
                .iter()
                .any(|(entity, _)| *entity == live.entity);
            if !kept {
                stage.remove_view(live.view);
            }
            kept
        });
    }

    /// The views to compose over the base, in the stack's order.
    pub fn drawn(&self, stack: &CameraStack) -> Vec<ViewId> {
        stack
            .overlays
            .iter()
            .filter_map(|(entity, _)| {
                self.live
                    .iter()
                    .find(|live| live.entity == *entity && live.drawn)
                    .map(|live| live.view)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
