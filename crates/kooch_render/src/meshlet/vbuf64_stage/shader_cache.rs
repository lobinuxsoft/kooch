//! A pipeline per surface shader, never per material (#1157).

use std::collections::HashMap;
use std::sync::Mutex;

use kooch_core::Guid;

use crate::material::{ShaderKind, SurfaceSource};
use crate::meshlet::validate_surface;

/// One shading path's custom-shader pipelines, keyed by shader and debug variant.
pub(crate) struct ShaderPipelines<P> {
    built: Mutex<HashMap<(Guid, bool), Built<P>>>,
    /// The kinds this path draws; any other is someone else's.
    kinds: &'static [ShaderKind],
}

/// What the opaque shading paths draw. A post-process is `PostPass`'s and composed as a surface has
/// no `sample_scene`; a transparent one is the forward pass's (#452).
pub(super) const OPAQUE: &[ShaderKind] = &[ShaderKind::Surface, ShaderKind::Unlit];

/// What the forward path draws: the two that blend, lit and not (#452).
pub(crate) const BLENDED: &[ShaderKind] = &[ShaderKind::Transparent, ShaderKind::TransparentUnlit];

struct Built<P> {
    /// The revision last tried, so a broken edit is reported once rather than every frame.
    revision: u64,
    /// The last pipeline that compiled. Kept through a broken edit.
    pipeline: Option<P>,
}

impl<P: Clone> ShaderPipelines<P> {
    pub(crate) fn new(kinds: &'static [ShaderKind]) -> Self {
        Self {
            built: Mutex::new(HashMap::new()),
            kinds,
        }
    }

    /// The pipeline for `surface`, building it when its revision moved. `None` means the shader
    /// has never compiled and the caller shades with the default.
    pub(crate) fn get(
        &self,
        guid: Guid,
        surface: &SurfaceSource,
        debug: bool,
        build: impl FnOnce(&SurfaceSource) -> P,
    ) -> Option<P> {
        if !self.kinds.contains(&surface.kind) {
            return None;
        }
        let mut built = self.built.lock().unwrap_or_else(|e| e.into_inner());
        let entry = built.entry((guid, debug)).or_insert(Built {
            revision: u64::MAX,
            pipeline: None,
        });
        if entry.revision != surface.revision {
            entry.revision = surface.revision;
            match validate_surface(&surface.params_wgsl, &surface.source) {
                Ok(()) => entry.pipeline = Some(build(surface)),
                Err(error) => tracing::error!(
                    target: "kooch_render::material::shader",
                    shader = %guid,
                    "shader does not compile, keeping the last good one — {error}",
                ),
            }
        }
        entry.pipeline.clone()
    }

    /// How many pipelines exist.
    #[cfg(test)]
    fn len(&self) -> usize {
        let built = self.built.lock().unwrap_or_else(|e| e.into_inner());
        built.values().filter(|b| b.pipeline.is_some()).count()
    }
}

#[cfg(test)]
mod tests;
