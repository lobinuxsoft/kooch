//! The UI Builder (#280 phase 8): a `.kxml` on a canvas of its own, at a chosen resolution, drawn by
//! the same pass that draws it in the game.

mod edit;
mod ghosts;
mod known;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_ecs::UiDrawList;
use kooch_ecs::query::Query;
use kooch_render::material::{ParamKind, Shader, ShaderParam};
use kooch_render::ui::UiPass;
use kooch_ui::markup::Markup;
use kooch_ui::taffy::{AvailableSpace, Size};
use kooch_ui::{CANVAS_HEIGHT, GlyphAtlas, UiDocument, UiFonts, UiShaders, UiTree};

use crate::viewport::ViewportTarget;

/// Screens a document is designed for, handheld first.
pub(crate) const RESOLUTIONS: &[(u32, u32, &str)] = &[
    (1280, 800, "Handheld 1280×800"),
    (1280, 720, "720p"),
    (1920, 1080, "1080p"),
    (2560, 1440, "1440p"),
    (1080, 1920, "Portrait 1080×1920"),
];

pub(crate) const SHEET_TYPE: &str = "kooch_ui::style::sheet::UiStyleSheet";
pub(crate) const SOURCE_TYPE: &str = "kooch_ui::source::UiSource";

/// The open document and its canvas. Taken out of `Resources` for the frame, like the views.
pub(crate) struct UiBuilder {
    pub path: PathBuf,
    pub source: Guid,
    pub style: Option<Guid>,
    pub resolution: (u32, u32),
    /// Screen pixels per canvas pixel.
    pub zoom: f32,
    /// A zoom the toolbar picked, applied around the canvas's centre.
    pub pending_zoom: Option<f32>,
    /// The canvas's top-left from the panel's, in points; `None` frames it.
    pub pan: Option<egui::Vec2>,
    pub focus_requested: bool,
    pub texture: Option<egui::TextureId>,
    /// The part of the canvas the panel shows; `None` while the tab is not drawn.
    pub request: Option<CanvasRegion>,
    /// The part the list was built for.
    shown: Option<CanvasRegion>,
    /// The part the target holds, where the panel paints it: a frame behind `shown` on a resize.
    pub painted: Option<CanvasRegion>,
    /// The file as edited, written only on save.
    markup: Markup,
    /// Differs from the file on disk.
    pub dirty: bool,
    /// The canvas has to be built again from `markup`.
    edited: bool,
    /// The file's and its sheet's mtimes when last read: a change on disk rebuilds.
    read_at: (SystemTime, SystemTime),
    pub selected: Option<u32>,
    /// What the canvas's materials read; the game's `load_shaders` never sees this tree.
    pub shaders: UiShaders,
    /// The last material the Inspector asked about, parsed once rather than per frame.
    pub material_params: Option<(Arc<str>, Vec<ShaderParam>)>,
    /// A named node of the selected `<use>`'s template: the Inspector edits its `<set>`.
    pub inner: Option<String>,
    /// Each `<use>` and its template's elements.
    ghosts: Vec<(u32, Vec<ghosts::Ghost>)>,
    /// Why the markup in memory does not build, shown over the canvas.
    pub error: Option<String>,
    tree: Option<UiTree>,
    pub known: known::Known,
    /// Every family `font-family` can name, by the font file holding it.
    pub fonts: Vec<(Guid, String)>,
    atlas: GlyphAtlas,
    list: UiDrawList,
    /// The size `list` was built for: the target matches it only once resized.
    drawn: Option<(u32, u32)>,
    target: Option<ViewportTarget>,
    pass: Option<UiPass>,
    /// What this frame's first edit replaced, filed once the panel hands the Builder back.
    step: Option<Step>,
    /// A bound control's value on the canvas, by element: the field it binds is the game's.
    previews: Vec<(u32, f32)>,
    /// A label's text on the canvas, by element: in the game, code writes it.
    texts: Vec<(u32, String)>,
}

/// What of the canvas is drawn, in its pixels at the current zoom: only what the panel shows, at the
/// pixels it is shown at, so a deep zoom draws vectors and text sharp instead of magnifying.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasRegion {
    /// The whole canvas.
    pub canvas: (u32, u32),
    /// Top-left of the part shown.
    pub offset: [f32; 2],
    pub size: (u32, u32),
}

/// One undo step: the markup before it, and what to call it.
pub(crate) struct Step {
    pub before: String,
    pub label: String,
    pub merge: Option<crate::history::MergeKey>,
}

impl UiBuilder {
    pub(crate) fn set_style(&mut self, style: Option<Guid>) {
        self.style = style;
        self.edited = true;
    }

    /// Before the UI: the target takes the size the panel asked for last frame.
    pub(crate) fn prepare(&mut self, gpu: &GpuContext, renderer: &mut egui_wgpu::Renderer) {
        if let Some(CanvasRegion { size, .. }) = self.request.take() {
            match self.target.as_mut() {
                Some(target) => {
                    target.request_size(size);
                    target.resize_if_needed(gpu.device(), renderer);
                }
                None => {
                    self.target = Some(ViewportTarget::new(
                        gpu.device(),
                        renderer,
                        gpu.format(),
                        size,
                    ))
                }
            }
        }
        self.texture = self.target.as_ref().map(ViewportTarget::texture_id);
    }

    /// What the material of the element at `origin` declares, textures aside.
    pub(crate) fn params_of(&mut self, origin: u32) -> &[ShaderParam] {
        let source = self
            .tree
            .as_ref()
            .and_then(|tree| tree.material(tree.node_of(origin)?))
            .and_then(|guid| self.shaders.source(guid));
        let Some(source) = source else {
            return &[];
        };
        if self.material_params.as_ref().map(|(own, _)| own) != Some(source) {
            let params = Shader::parse(source)
                .map(|shader| shader.params)
                .unwrap_or_default()
                .into_iter()
                .filter(|param| param.kind != ParamKind::Texture)
                .collect();
            self.material_params = Some((source.clone(), params));
        }
        self.material_params
            .as_ref()
            .map_or(&[], |(_, params)| params.as_slice())
    }

    /// Over transparent black: the panel's checkerboard shows through.
    pub(crate) fn render(&mut self, gpu: &GpuContext) {
        let (Some(target), Some(drawn), Some(shown)) =
            (self.target.as_ref(), self.drawn, self.shown)
        else {
            return;
        };
        if target.size() != drawn {
            return;
        }
        let pass = self
            .pass
            .get_or_insert_with(|| UiPass::new(gpu.device(), gpu.format()));
        pass.set_offset(shown.offset);
        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ui_builder"),
            });
        pass.draw(
            gpu.device(),
            gpu.queue(),
            &mut encoder,
            target.view(),
            drawn,
            &mut self.list,
            true,
        );
        gpu.queue().submit(Some(encoder.finish()));
        self.painted = Some(shown);
    }

    fn update(&mut self, resources: &mut Resources) {
        let Some(region) = self.request else {
            return;
        };
        let (size, canvas_pixels) = (region.size, region.canvas);
        let mut server = resources.remove::<AssetServer>();
        if let Some(database) = resources.get::<AssetDatabase>() {
            self.reread(database);
            if std::mem::take(&mut self.edited) || self.tree.is_none() {
                let text = self.markup.write();
                self.known
                    .refresh(database, server.as_mut(), &text, &self.path, self.style);
                self.ghosts = ghosts::of(&self.markup, &self.path);
                match kooch_ui::build_markup(
                    database,
                    server.as_mut(),
                    &text,
                    &self.path,
                    self.style,
                ) {
                    Ok(mut tree) => {
                        // Written once per build, not per frame: reshaping text is not free.
                        for (origin, text) in &self.texts {
                            if let Some(node) = tree.node_of(*origin) {
                                tree.write_text(node, text);
                            }
                        }
                        self.tree = Some(tree);
                        self.error = None;
                    }
                    // The last tree that built stays up, under the error.
                    Err(error) => self.error = Some(error),
                }
            }
            // What the panel shows: a vector past the largest size is drawn only there.
            let [x, y] = region.offset;
            let view = [x, y, x + size.0 as f32, y + size.1 as f32];
            if let Some(tree) = self.tree.as_mut() {
                tree.set_view(Some(view));
            }
            // The canvas draws with its own atlas, which the game's `load_images` never fills.
            if let Some(tree) = self.tree.as_ref() {
                let scale = canvas_pixels.1.max(1) as f32 / CANVAS_HEIGHT;
                kooch_ui::load_tree_images(tree, scale, database, server.as_mut(), &mut self.atlas);
                kooch_ui::load_tree_shaders(tree, database, server.as_mut(), &mut self.shaders);
            }
        }
        if let Some(server) = server {
            resources.insert(server);
        }
        if let Some(fonts) = resources.get::<UiFonts>()
            && fonts.families().len() != self.fonts.len()
        {
            self.fonts = fonts
                .families()
                .iter()
                .map(|(guid, name)| (*guid, name.to_string()))
                .collect();
        }
        let (Some(tree), Some(fonts)) = (self.tree.as_mut(), resources.get_mut::<UiFonts>()) else {
            return;
        };
        for &(origin, value) in &self.previews {
            if let Some(node) = tree.node_of(origin) {
                tree.set_value(node, value);
            }
        }
        let canvas = kooch_ui::canvas(glam::UVec2::new(self.resolution.0, self.resolution.1));
        tree.layout(
            Size {
                width: AvailableSpace::Definite(canvas.x),
                height: AvailableSpace::Definite(canvas.y),
            },
            fonts,
        );
        let scale = canvas_pixels.1.max(1) as f32 / CANVAS_HEIGHT;
        tree.rasterize(scale, fonts, &mut self.atlas);
        let physical = glam::UVec2::new(canvas_pixels.0, canvas_pixels.1);
        kooch_ui::draw_tree(
            tree,
            &mut self.atlas,
            &self.shaders,
            scale,
            physical,
            &mut self.list,
        );
        self.drawn = Some(size);
        self.shown = Some(region);
    }
}

/// Opens `path` in the Builder, wearing the sheet a document in the scene gives it.
pub(crate) fn open(resources: &mut Resources, path: &Path) {
    let Some(source) = resources
        .get::<AssetDatabase>()
        .and_then(|database| database.guid_for(path))
    else {
        tracing::error!(file = %path.display(), "not a known asset; cannot open it in the UI Builder");
        return;
    };
    let markup = match std::fs::read_to_string(path)
        .map_err(|error| error.to_string())
        .and_then(|text| Markup::parse(&text))
    {
        Ok(markup) => markup,
        Err(error) => {
            tracing::error!(file = %path.display(), %error, "cannot open it in the UI Builder");
            return;
        }
    };
    if let Some(builder) = resources.get_mut::<UiBuilder>()
        && builder.source == source
    {
        builder.focus_requested = true;
        return;
    }
    let mut style = None;
    Query::<&UiDocument>::new(resources).for_each(|document| {
        if style.is_none() && document.source == Some(source) {
            style = document.style;
        }
    });
    // Another file reuses the canvas's GPU objects and glyphs.
    let old = resources.remove::<UiBuilder>();
    if let Some(old) = old.as_ref().filter(|old| old.dirty) {
        tracing::warn!(file = %old.path.display(), "closed in the UI Builder with unsaved changes");
    }
    let (atlas, target, pass) = match old {
        Some(old) => (old.atlas, old.target, old.pass),
        None => (GlyphAtlas::default(), None, None),
    };
    resources.insert(UiBuilder {
        path: path.to_path_buf(),
        source,
        style,
        resolution: (RESOLUTIONS[0].0, RESOLUTIONS[0].1),
        zoom: 1.0,
        pending_zoom: None,
        pan: None,
        focus_requested: true,
        texture: None,
        request: None,
        shown: None,
        painted: None,
        markup,
        dirty: false,
        edited: true,
        read_at: (SystemTime::UNIX_EPOCH, SystemTime::UNIX_EPOCH),
        selected: None,
        shaders: UiShaders::default(),
        material_params: None,
        inner: None,
        ghosts: Vec::new(),
        error: None,
        tree: None,
        known: known::Known::default(),
        fonts: Vec::new(),
        atlas,
        list: UiDrawList::default(),
        drawn: None,
        target,
        pass,
        step: None,
        previews: Vec::new(),
        texts: Vec::new(),
    });
}

/// Before the render: the open document, built when its files change and laid out at the canvas.
pub(crate) fn builder_system(resources: &mut Resources) {
    let Some(mut builder) = resources.remove::<UiBuilder>() else {
        return;
    };
    builder.update(resources);
    resources.insert(builder);
}
