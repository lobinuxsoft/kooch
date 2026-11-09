//! The vector atlas (#1865): SVGs and text drawn by Vello into one texture the quads sample. Each
//! new entry is drawn alone and copied into its slot (#1880): Vello's buffers are fixed, and the
//! whole atlas in one scene overflows them into a blank texture. Without `vello` it stays 1×1.

use kooch_ecs::UiDrawList;
#[cfg(feature = "vello")]
use kooch_ecs::UiVector;

pub(super) struct UiVectors {
    #[cfg_attr(not(feature = "vello"), allow(dead_code))]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    #[cfg(feature = "vello")]
    side: u32,
    #[cfg(feature = "vello")]
    vello: drawn::Drawn,
}

impl UiVectors {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let texture = texture(device, 1);
        Self {
            view: texture.create_view(&Default::default()),
            texture,
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
        let texts = std::mem::take(&mut list.vector_texts);
        let fonts = std::mem::take(&mut list.vector_fonts);
        #[cfg(not(feature = "vello"))]
        {
            let _ = (device, queue, sources, texts, fonts);
            false
        }
        #[cfg(feature = "vello")]
        {
            self.vello.parse(sources);
            self.vello.parse_texts(texts, fonts);
            let replaced = list.vector_side > 0 && list.vector_side != self.side;
            if replaced {
                let grown = texture(device, list.vector_side);
                // Slots keep their place when the atlas grows: what is drawn is carried over.
                match list.vector_side > self.side {
                    true => copy_into(device, queue, &self.texture, &grown, self.side),
                    false => self.vello.forget(),
                }
                self.view = grown.create_view(&Default::default());
                self.texture = grown;
                self.side = list.vector_side;
            }
            self.vello.draw(device, queue, &self.texture, &list.vectors);
            replaced
        }
    }
}

#[cfg(feature = "vello")]
mod drawn {
    use std::collections::HashMap;
    use std::num::NonZeroUsize;
    use std::sync::{Mutex, PoisonError};

    use kooch_ecs::{UiFontSource, UiVectorSource, UiVectorText};
    use vello_svg::vello::kurbo::Affine;
    use vello_svg::vello::peniko::{Blob, Color, Fill, FontData};
    use vello_svg::vello::{
        AaConfig, AaSupport, Glyph, RenderParams, Renderer, RendererOptions, Scene,
    };

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
        /// Every font a text used, kept: a later text may use one sent long before.
        fonts: HashMap<u64, Blob<u8>>,
        scene: Scene,
        /// The slot each entry was drawn into; one missing or moved is drawn.
        drawn: HashMap<u64, [u32; 4]>,
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

        /// Encodes each new text's glyph runs, white: the quad tints them with the text colour.
        pub(super) fn parse_texts(&mut self, texts: Vec<UiVectorText>, fonts: Vec<UiFontSource>) {
            for font in fonts {
                self.fonts
                    .entry(font.id)
                    .or_insert_with(|| Blob::new(std::sync::Arc::new(font.data)));
            }
            for text in texts {
                if self.fragments.contains_key(&text.key) {
                    continue;
                }
                let mut scene = Scene::new();
                for run in &text.runs {
                    let Some(blob) = self.fonts.get(&run.font) else {
                        tracing::warn!("UI text not drawn: its font never arrived");
                        continue;
                    };
                    let font = FontData::new(blob.clone(), run.index);
                    let glyphs = run.glyphs.iter().map(|&(id, [x, y])| Glyph { id, x, y });
                    scene
                        .draw_glyphs(&font)
                        .font_size(run.size)
                        .normalized_coords(&run.coords)
                        // As swash did: small text keeps its stems on whole pixels.
                        .hint(true)
                        .brush(Color::WHITE)
                        .draw(Fill::NonZero, glyphs);
                }
                let fragment = Fragment {
                    scene,
                    size: text.size.map(f64::from),
                };
                self.fragments.insert(text.key, fragment);
            }
        }

        pub(super) fn forget(&mut self) {
            self.drawn.clear();
        }

        pub(super) fn draw(
            &mut self,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            atlas: &wgpu::Texture,
            vectors: &[UiVector],
        ) {
            let stale = |vector: &&UiVector| self.drawn.get(&vector.key) != Some(&vector.rect);
            if !vectors.iter().any(|vector| stale(&vector)) {
                return;
            }
            profiling::scope!("ui: vello");
            let drawing = |key: &u64| vectors.iter().any(|vector| vector.source == *key);
            self.fragments.retain(|key, _| drawing(key));
            let resident = |key: &u64| vectors.iter().any(|vector| vector.key == *key);
            self.drawn.retain(|key, _| resident(key));
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
            let renderer = renderer.get_mut().unwrap_or_else(PoisonError::into_inner);
            for vector in vectors {
                if self.drawn.get(&vector.key) == Some(&vector.rect) {
                    continue;
                }
                // Not arrived yet: drawn the frame it does.
                let Some(fragment) = self.fragments.get(&vector.source) else {
                    continue;
                };
                let [x0, y0, x1, y1] = vector.rect;
                let (w, h) = (x1 - x0, y1 - y0);
                // A crop draws the whole at its full size, moved so its part lands in the slot.
                let [full_w, full_h, from_x, from_y] = vector.crop.map(f64::from);
                let (whole, from) = match full_w > 0.0 {
                    true => ([full_w, full_h], [from_x, from_y]),
                    false => ([f64::from(w), f64::from(h)], [0.0, 0.0]),
                };
                let scale = [
                    whole[0] / fragment.size[0].max(f64::EPSILON),
                    whole[1] / fragment.size[1].max(f64::EPSILON),
                ];
                // One texel of margin, drawn empty: a reused slot loses what was around it.
                self.scene.reset();
                let place = Affine::translate((1.0 - from[0], 1.0 - from[1]))
                    * Affine::scale_non_uniform(scale[0], scale[1]);
                self.scene.append(&fragment.scene, Some(place));
                let scratch = super::scratch(device, w + 2, h + 2);
                let params = RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: w + 2,
                    height: h + 2,
                    antialiasing_method: AaConfig::Area,
                };
                let view = scratch.create_view(&Default::default());
                if let Err(error) =
                    renderer.render_to_texture(device, queue, &self.scene, &view, &params)
                {
                    tracing::warn!("UI vector not drawn: {error}");
                    continue;
                }
                super::copy_slot(device, queue, &scratch, atlas, [x0 - 1, y0 - 1]);
                self.drawn.insert(vector.key, vector.rect);
            }
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

/// Rgba8Unorm, straight alpha, as the image atlas is. Copied into, and out of when it grows.
fn texture(device: &wgpu::Device, side: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui_vectors"),
        size: extent(side, side),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Where Vello draws one entry: Rgba8Unorm is the one storage format it writes.
#[cfg(feature = "vello")]
fn scratch(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui_vector_scratch"),
        size: extent(w, h),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// All of `from` into `to` at `at`.
#[cfg(feature = "vello")]
fn copy_slot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    from: &wgpu::Texture,
    to: &wgpu::Texture,
    [x, y]: [u32; 2],
) {
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_texture(
        from.as_image_copy(),
        wgpu::TexelCopyTextureInfo {
            texture: to,
            mip_level: 0,
            origin: wgpu::Origin3d { x, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        from.size(),
    );
    queue.submit([encoder.finish()]);
}

/// The top-left `side`×`side` of `from` into `to`.
#[cfg(feature = "vello")]
fn copy_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    from: &wgpu::Texture,
    to: &wgpu::Texture,
    side: u32,
) {
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_texture(from.as_image_copy(), to.as_image_copy(), extent(side, side));
    queue.submit([encoder.finish()]);
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}
