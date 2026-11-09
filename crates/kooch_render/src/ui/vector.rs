//! The vector atlas (#1865): SVGs and text drawn by Vello into one texture the quads sample. New
//! entries are packed into a kept scratch texture, drawn in one Vello pass per scratch and copied
//! into their slots on the frame's encoder (#1880). Vello's buffers are fixed: the whole atlas in
//! one scene overflows them into a blank texture, so a pass covers one scratch at most.
//! Without `vello` it stays 1×1.

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
        encoder: &mut wgpu::CommandEncoder,
        list: &mut UiDrawList,
    ) -> bool {
        let sources = std::mem::take(&mut list.vector_sources);
        let texts = std::mem::take(&mut list.vector_texts);
        let fonts = std::mem::take(&mut list.vector_fonts);
        #[cfg(not(feature = "vello"))]
        {
            let _ = (device, queue, encoder, sources, texts, fonts);
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
                    true => copy_into(encoder, &self.texture, &grown, self.side),
                    false => self.vello.forget(),
                }
                self.view = grown.create_view(&Default::default());
                self.texture = grown;
                self.side = list.vector_side;
            }
            self.vello
                .draw(device, queue, encoder, &self.texture, &list.vectors);
            replaced
        }
    }
}

#[cfg(feature = "vello")]
mod drawn {
    use std::collections::HashMap;
    use std::num::NonZeroUsize;
    use std::sync::{Mutex, PoisonError};

    use kooch_ecs::{UiFontSource, UiGlyphRun, UiVectorSource, UiVectorText};
    use vello_svg::vello::kurbo::{Affine, Rect};
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
        /// Kept between frames: one per pass a frame needed so far.
        scratches: Vec<wgpu::Texture>,
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
                    if !run.turns.is_empty() {
                        draw_turned(&mut scene, &font, run);
                        continue;
                    }
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
            encoder: &mut wgpu::CommandEncoder,
            atlas: &wgpu::Texture,
            vectors: &[UiVector],
        ) {
            let stale = |drawn: &HashMap<u64, [u32; 4]>, vector: &UiVector| {
                drawn.get(&vector.key) != Some(&vector.rect)
            };
            if !vectors.iter().any(|vector| stale(&self.drawn, vector)) {
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
            // Not arrived yet: drawn the frame it does.
            let mut pending: Vec<&UiVector> = vectors
                .iter()
                .filter(|v| stale(&self.drawn, v) && self.fragments.contains_key(&v.source))
                .collect();
            pending
                .sort_unstable_by_key(|vector| std::cmp::Reverse(vector.rect[3] - vector.rect[1]));
            for batch in 0.. {
                let (shelved, used) = shelve(&mut pending);
                if shelved.is_empty() {
                    break;
                }
                self.scene.reset();
                for &(vector, at) in &shelved {
                    let Some(fragment) = self.fragments.get(&vector.source) else {
                        continue;
                    };
                    place(&mut self.scene, fragment, vector, at);
                }
                let own;
                let scratch = match used[0] <= SCRATCH && used[1] <= SCRATCH {
                    true => {
                        while self.scratches.len() <= batch {
                            self.scratches
                                .push(super::scratch(device, SCRATCH, SCRATCH));
                        }
                        &self.scratches[batch]
                    }
                    false => {
                        own = super::scratch(device, used[0], used[1]);
                        &own
                    }
                };
                let params = RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: used[0],
                    height: used[1],
                    antialiasing_method: AaConfig::Area,
                };
                let view = scratch.create_view(&Default::default());
                if let Err(error) =
                    renderer.render_to_texture(device, queue, &self.scene, &view, &params)
                {
                    tracing::warn!("UI vectors not drawn: {error}");
                    continue;
                }
                // Recorded after Vello's submit, so they read what it drew.
                for (vector, at) in shelved {
                    let [x0, y0, x1, y1] = vector.rect;
                    let size = [x1 - x0 + 2, y1 - y0 + 2];
                    super::copy_slot(encoder, scratch, atlas, at, [x0 - 1, y0 - 1], size);
                    self.drawn.insert(vector.key, vector.rect);
                }
            }
        }
    }

    /// Side of each kept scratch texture: a pass over it stays well inside Vello's buffers.
    const SCRATCH: u32 = 1024;

    /// Takes from `pending`, tallest first, what fits one scratch in rows, with the corner each
    /// lands at and the extent used. One larger than a scratch goes alone, at its own size.
    fn shelve<'a>(pending: &mut Vec<&'a UiVector>) -> (Vec<(&'a UiVector, [u32; 2])>, [u32; 2]) {
        let cell = |vector: &UiVector| {
            let [x0, y0, x1, y1] = vector.rect;
            [x1 - x0 + 2, y1 - y0 + 2]
        };
        if let Some(index) = pending
            .iter()
            .position(|v| cell(v).iter().any(|&s| s > SCRATCH))
        {
            let vector = pending.remove(index);
            return (vec![(vector, [0, 0])], cell(vector));
        }
        let (mut shelved, mut left) = (Vec::new(), Vec::new());
        let ([mut x, mut y], mut row, mut used) = ([0, 0], 0, [0, 0]);
        for vector in pending.drain(..) {
            let [w, h] = cell(vector);
            if x + w > SCRATCH {
                (x, y, row) = (0, y + row, 0);
            }
            if y + h > SCRATCH {
                left.push(vector);
                continue;
            }
            shelved.push((vector, [x, y]));
            used = [used[0].max(x + w), used[1].max(y + h)];
            (x, row) = (x + w, row.max(h));
        }
        *pending = left;
        (shelved, used)
    }

    /// `vector`'s drawing with its cell's corner at `at`, cut to its slot so nothing reaches a
    /// neighbour. A crop draws the whole at its full size, moved so its part lands in the slot.
    fn place(scene: &mut Scene, fragment: &Fragment, vector: &UiVector, at: [u32; 2]) {
        let [x0, y0, x1, y1] = vector.rect;
        let (w, h) = (f64::from(x1 - x0), f64::from(y1 - y0));
        let [full_w, full_h, from_x, from_y] = vector.crop.map(f64::from);
        let (whole, from) = match full_w > 0.0 {
            true => ([full_w, full_h], [from_x, from_y]),
            false => ([w, h], [0.0, 0.0]),
        };
        let scale = [
            whole[0] / fragment.size[0].max(f64::EPSILON),
            whole[1] / fragment.size[1].max(f64::EPSILON),
        ];
        // One texel of margin, left empty: the filter reads past the slot's edge.
        let corner = [f64::from(at[0]) + 1.0, f64::from(at[1]) + 1.0];
        let slot = Rect::new(corner[0], corner[1], corner[0] + w, corner[1] + h);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &slot);
        let moved = Affine::translate((corner[0] - from[0], corner[1] - from[1]))
            * Affine::scale_non_uniform(scale[0], scale[1]);
        scene.append(&fragment.scene, Some(moved));
        scene.pop_layer();
    }

    /// One glyph at a time, each turned about its pen position: text set along a curve.
    fn draw_turned(scene: &mut Scene, font: &FontData, run: &UiGlyphRun) {
        for (&(id, [x, y]), &[cos, sin]) in run.glyphs.iter().zip(&run.turns) {
            let turn = [cos, sin, -sin, cos, x, y].map(f64::from);
            scene
                .draw_glyphs(font)
                .font_size(run.size)
                .normalized_coords(&run.coords)
                .transform(Affine::new(turn))
                .brush(Color::WHITE)
                .draw(Fill::NonZero, std::iter::once(Glyph { id, x: 0.0, y: 0.0 }));
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

/// Where Vello draws a pass: Rgba8Unorm is the one storage format it writes.
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

/// `size` texels of `from` at `at` into `to` at `into`.
#[cfg(feature = "vello")]
fn copy_slot(
    encoder: &mut wgpu::CommandEncoder,
    from: &wgpu::Texture,
    to: &wgpu::Texture,
    [x, y]: [u32; 2],
    [to_x, to_y]: [u32; 2],
    [w, h]: [u32; 2],
) {
    let origin = |x, y| wgpu::Origin3d { x, y, z: 0 };
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            origin: origin(x, y),
            ..from.as_image_copy()
        },
        wgpu::TexelCopyTextureInfo {
            origin: origin(to_x, to_y),
            ..to.as_image_copy()
        },
        extent(w, h),
    );
}

/// The top-left `side`×`side` of `from` into `to`.
#[cfg(feature = "vello")]
fn copy_into(
    encoder: &mut wgpu::CommandEncoder,
    from: &wgpu::Texture,
    to: &wgpu::Texture,
    side: u32,
) {
    encoder.copy_texture_to_texture(from.as_image_copy(), to.as_image_copy(), extent(side, side));
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}
