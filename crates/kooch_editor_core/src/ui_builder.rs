//! The UI Builder (#280 phase 8): a `.kxml` on a canvas of its own, at a chosen resolution, drawn by
//! the same pass that draws it in the game.

mod edit;
mod known;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_ecs::UiDrawList;
use kooch_ecs::query::Query;
use kooch_render::ui::UiPass;
use kooch_ui::markup::Markup;
use kooch_ui::taffy::{AvailableSpace, Size};
use kooch_ui::{CANVAS_HEIGHT, GlyphAtlas, UiDocument, UiFonts, UiTree};

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
    /// Pixels the panel shows the canvas at; `None` while the tab is not drawn.
    pub request: Option<(u32, u32)>,
    /// The file as edited, written only on save.
    markup: Markup,
    /// Differs from the file on disk.
    pub dirty: bool,
    /// The canvas has to be built again from `markup`.
    edited: bool,
    /// The file's and its sheet's mtimes when last read: a change on disk rebuilds.
    read_at: (SystemTime, SystemTime),
    pub selected: Option<u32>,
    /// Why the markup in memory does not build, shown over the canvas.
    pub error: Option<String>,
    tree: Option<UiTree>,
    pub known: known::Known,
    atlas: GlyphAtlas,
    list: UiDrawList,
    /// The size `list` was built for: the target matches it only once resized.
    drawn: Option<(u32, u32)>,
    target: Option<ViewportTarget>,
    pass: Option<UiPass>,
}

impl UiBuilder {
    pub(crate) fn set_style(&mut self, style: Option<Guid>) {
        self.style = style;
        self.edited = true;
    }

    /// Before the UI: the target takes the size the panel asked for last frame.
    pub(crate) fn prepare(&mut self, gpu: &GpuContext, renderer: &mut egui_wgpu::Renderer) {
        if let Some(size) = self.request.take() {
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

    /// Over transparent black: the panel's checkerboard shows through.
    pub(crate) fn render(&mut self, gpu: &GpuContext) {
        let (Some(target), Some(drawn)) = (self.target.as_ref(), self.drawn) else {
            return;
        };
        if target.size() != drawn {
            return;
        }
        let pass = self
            .pass
            .get_or_insert_with(|| UiPass::new(gpu.device(), gpu.format()));
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
    }

    fn update(&mut self, resources: &mut Resources) {
        let Some(size) = self.request else {
            return;
        };
        let mut server = resources.remove::<AssetServer>();
        if let Some(database) = resources.get::<AssetDatabase>() {
            self.reread(database);
            if std::mem::take(&mut self.edited) || self.tree.is_none() {
                let text = self.markup.write();
                self.known.refresh(database, &text, self.style);
                match kooch_ui::build_markup(
                    database,
                    server.as_mut(),
                    &text,
                    &self.path,
                    self.style,
                ) {
                    Ok(tree) => {
                        self.tree = Some(tree);
                        self.error = None;
                    }
                    // The last tree that built stays up, under the error.
                    Err(error) => self.error = Some(error),
                }
            }
        }
        if let Some(server) = server {
            resources.insert(server);
        }
        let (Some(tree), Some(fonts)) = (self.tree.as_mut(), resources.get_mut::<UiFonts>()) else {
            return;
        };
        let canvas = kooch_ui::canvas(glam::UVec2::new(self.resolution.0, self.resolution.1));
        tree.layout(
            Size {
                width: AvailableSpace::Definite(canvas.x),
                height: AvailableSpace::Definite(canvas.y),
            },
            fonts,
        );
        tree.rasterize(size.1.max(1) as f32 / CANVAS_HEIGHT, fonts, &mut self.atlas);
        let physical = glam::UVec2::new(size.0, size.1);
        kooch_ui::draw_tree(tree, &mut self.atlas, physical, &mut self.list);
        self.drawn = Some(size);
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
        markup,
        dirty: false,
        edited: true,
        read_at: (SystemTime::UNIX_EPOCH, SystemTime::UNIX_EPOCH),
        selected: None,
        error: None,
        tree: None,
        known: known::Known::default(),
        atlas,
        list: UiDrawList::default(),
        drawn: None,
        target,
        pass,
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
