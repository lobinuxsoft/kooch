//! The vector atlas (#1865): every resident SVG drawn by Vello into one texture the quads sample,
//! redrawn whole only when what is resident changes. Without the `vello` feature it stays 1×1.

use kooch_ecs::UiDrawList;
#[cfg(feature = "vello")]
use kooch_ecs::UiVector;

pub(super) struct UiVectors {
    /// Keeps its texture alive.
    view: wgpu::TextureView,
    #[cfg(feature = "vello")]
    side: u32,
    #[cfg(feature = "vello")]
    vello: drawn::Drawn,
}

impl UiVectors {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            view: texture(device, 1),
            #[cfg(feature = "vello")]
            side: 1,
            #[cfg(feature = "vello")]
            vello: drawn::Drawn::default(),
        }
    }

    pub(super) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Draws what changed. True when the texture was replaced, so the bind group is stale.
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        list: &mut UiDrawList,
    ) -> bool {
        let sources = std::mem::take(&mut list.vector_sources);
        #[cfg(not(feature = "vello"))]
        {
            let _ = (device, queue, sources);
            false
        }
        #[cfg(feature = "vello")]
        {
            self.vello.parse(sources);
            let replaced = list.vector_side > 0 && list.vector_side != self.side;
            if replaced {
                self.view = texture(device, list.vector_side);
                self.side = list.vector_side;
                self.vello.forget();
            }
            self.vello
                .draw(device, queue, &self.view, self.side, &list.vectors);
            replaced
        }
    }
}

#[cfg(feature = "vello")]
mod drawn {
    use std::collections::HashMap;
    use std::num::NonZeroUsize;
    use std::sync::{Mutex, PoisonError};

    use kooch_ecs::UiVectorSource;
    use vello_svg::vello::kurbo::{Affine, Rect};
    use vello_svg::vello::peniko::{Color, Fill};
    use vello_svg::vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

    use super::UiVector;

    /// Each file encoded once, with the size it was drawn at.
    struct Fragment {
        scene: Scene,
        size: [f64; 2],
    }

    #[derive(Default)]
    pub(super) struct Drawn {
        /// Made on the first vector, so a UI without one never builds Vello's pipelines. In a
        /// mutex only because it is not `Sync` and resources must be; `get_mut` never locks.
        renderer: Option<Mutex<Renderer>>,
        fragments: HashMap<u64, Fragment>,
        scene: Scene,
        /// What the texture holds; `None` until it holds the list, or while a file is missing so
        /// its arrival redraws.
        drawn: Option<Vec<UiVector>>,
    }

    impl Drawn {
        pub(super) fn parse(&mut self, sources: Vec<UiVectorSource>) {
            for source in sources {
                if self.fragments.contains_key(&source.key) {
                    continue;
                }
                let tree = match usvg_tree(&source.svg) {
                    Ok(tree) => tree,
                    Err(error) => {
                        tracing::warn!("UI vector not drawn: {error}");
                        continue;
                    }
                };
                let mut scene = Scene::new();
                // No red box over what Vello cannot draw: a shipped UI shows nothing instead.
                vello_svg::append_tree_with(&mut scene, &tree, &mut |_, _| {});
                let size = tree.size();
                let fragment = Fragment {
                    scene,
                    size: [f64::from(size.width()), f64::from(size.height())],
                };
                self.fragments.insert(source.key, fragment);
            }
        }

        pub(super) fn forget(&mut self) {
            self.drawn = None;
        }

        pub(super) fn draw(
            &mut self,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            view: &wgpu::TextureView,
            side: u32,
            vectors: &[UiVector],
        ) {
            if vectors.is_empty() || self.drawn.as_deref() == Some(vectors) {
                return;
            }
            profiling::scope!("ui: vello");
            self.fragments
                .retain(|key, _| vectors.iter().any(|vector| vector.key == *key));
            let renderer = match &mut self.renderer {
                Some(renderer) => renderer,
                slot => match Renderer::new(device, options()) {
                    Ok(renderer) => slot.insert(Mutex::new(renderer)),
                    Err(error) => {
                        tracing::warn!("UI vectors not drawn: Vello did not start: {error}");
                        return;
                    }
                },
            };
            self.scene.reset();
            let mut complete = true;
            for vector in vectors {
                let Some(fragment) = self.fragments.get(&vector.key) else {
                    complete = false;
                    continue;
                };
                let [x0, y0, x1, y1] = vector.rect.map(f64::from);
                let scale = [
                    (x1 - x0) / fragment.size[0].max(f64::EPSILON),
                    (y1 - y0) / fragment.size[1].max(f64::EPSILON),
                ];
                // Cut to its slot: a path past the view box would land in a neighbour.
                let slot = Rect::new(x0, y0, x1, y1);
                self.scene
                    .push_clip_layer(Fill::NonZero, Affine::IDENTITY, &slot);
                let place =
                    Affine::translate((x0, y0)) * Affine::scale_non_uniform(scale[0], scale[1]);
                self.scene.append(&fragment.scene, Some(place));
                self.scene.pop_layer();
            }
            let params = RenderParams {
                base_color: Color::TRANSPARENT,
                width: side,
                height: side,
                antialiasing_method: AaConfig::Area,
            };
            let renderer = renderer.get_mut().unwrap_or_else(PoisonError::into_inner);
            if let Err(error) =
                renderer.render_to_texture(device, queue, &self.scene, view, &params)
            {
                tracing::warn!("UI vectors not drawn: {error}");
                return;
            }
            self.drawn = complete.then(|| vectors.to_vec());
        }
    }

    fn usvg_tree(svg: &[u8]) -> Result<vello_svg::usvg::Tree, String> {
        let options = vello_svg::usvg::Options::default();
        vello_svg::usvg::Tree::from_data(svg, &options).map_err(|e| format!("SVG: {e}"))
    }

    /// Area AA only: the other modes' pipelines cost start-up time and the atlas does not use them.
    fn options() -> RendererOptions {
        RendererOptions {
            use_cpu: false,
            antialiasing_support: AaSupport::area_only(),
            num_init_threads: NonZeroUsize::new(1),
            pipeline_cache: None,
        }
    }
}

/// Rgba8Unorm, the one storage format Vello writes; straight alpha, as the image atlas is.
fn texture(device: &wgpu::Device, side: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui_vectors"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&Default::default())
}
