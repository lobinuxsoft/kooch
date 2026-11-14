//! The host's UI, carried to the editor in remote Play (#1691): the host lays it out but has no
//! renderer, so the editor's Game View draws it.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::{
    AtlasUpload, UiCurve, UiDrawList, UiFontSource, UiGlyphRun, UiMask, UiMaterial, UiMirror,
    UiParam, UiQuad, UiShader, UiVector, UiVectorSource, UiVectorText, UiVideo, UiVideoSource,
};
use serde::{Deserialize, Serialize};

/// A [`UiDrawList`] on the wire. Bytes as base64: a quad is 104 bytes, and as JSON numbers it
/// would be three times that, every frame the UI changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiFrame {
    /// `UiQuad`s, back to front.
    pub quads: String,
    /// `UiMask`s, which the quads index.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub masks: String,
    /// `UiCurve`s, which the quads index.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub curves: String,
    /// `UiMirror`s, which the quads index.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mirrors: String,
    /// `quad`, `shader`, first param, param count.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<[u32; 4]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<(String, [f32; 4])>,
    /// Only when they changed: a bar that moves every frame sends its value, not its shader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shaders: Option<Vec<(Guid, String)>>,
    pub atlas_side: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<UiUpload>,
    /// The vector atlas (#1865): its side, `(key, rect, source, crop)` per slot, each file once.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub vector_side: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vectors: Vec<(u64, [u32; 4], u64, [f32; 4])>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vector_sources: Vec<(u64, String)>,
    /// Text slots' glyphs (#1880), and each font file once, base64.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vector_texts: Vec<WireText>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vector_fonts: Vec<(u64, String)>,
    /// Videos shown, `(key, slot)`, and each WebM once, base64 (#1793): the editor plays them
    /// itself, so no frame crosses the wire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub videos: Vec<(u64, [u32; 4])>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub video_sources: Vec<(u64, String)>,
    /// The page atlas (#1893): its side, and each large raster once, as it was packed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub page_side: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub page_uploads: Vec<UiUpload>,
}

/// A [`UiVectorText`] on the wire.
/// A [`UiGlyphRun`] on the wire.
pub type WireRun = (u64, u32, f32, Vec<i16>, Vec<(u32, [f32; 2])>, Vec<[f32; 2]>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireText {
    pub key: u64,
    pub size: [f32; 2],
    /// Font, face, pixels per em, coordinates, glyphs, their turns.
    pub runs: Vec<WireRun>,
}

impl From<&UiVectorText> for WireText {
    fn from(text: &UiVectorText) -> Self {
        let runs = text.runs.iter().map(|run| {
            let coords = run.coords.clone();
            let turns = run.turns.clone();
            (
                run.font,
                run.index,
                run.size,
                coords,
                run.glyphs.clone(),
                turns,
            )
        });
        Self {
            key: text.key,
            size: text.size,
            runs: runs.collect(),
        }
    }
}

impl From<WireText> for UiVectorText {
    fn from(wire: WireText) -> Self {
        let runs = wire
            .runs
            .into_iter()
            .map(|(font, index, size, coords, glyphs, turns)| UiGlyphRun {
                font,
                index,
                size,
                coords,
                glyphs,
                turns,
            });
        Self {
            key: wire.key,
            size: wire.size,
            runs: runs.collect(),
        }
    }
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// An [`AtlasUpload`] on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiUpload {
    pub rect: [u32; 4],
    pub texels: String,
}

/// What was last sent, so a UI that did not change costs nothing on the wire.
#[derive(Default)]
struct SentUi {
    quads: Vec<UiQuad>,
    masks: Vec<UiMask>,
    curves: Vec<UiCurve>,
    mirrors: Vec<UiMirror>,
    materials: Vec<UiMaterial>,
    params: Vec<UiParam>,
    shaders: Vec<UiShader>,
    vectors: Vec<UiVector>,
    videos: Vec<UiVideo>,
}

/// This frame's UI when it changed since the last reply. Takes the atlas upload: the editor keeps
/// what it was sent, so each texel crosses once.
pub(crate) fn take(resources: &mut Resources) -> Option<UiFrame> {
    let mut sent = resources.remove::<SentUi>().unwrap_or_default();
    let frame = resources.get_mut::<UiDrawList>().and_then(|list| {
        // A host merges its uploads into one, grown over its own atlas.
        let upload = list.atlas_uploads.pop();
        list.atlas_uploads.clear();
        let pages = std::mem::take(&mut list.page_uploads);
        let videos = std::mem::take(&mut list.video_sources);
        let sources = std::mem::take(&mut list.vector_sources);
        let texts = std::mem::take(&mut list.vector_texts);
        let fonts = std::mem::take(&mut list.vector_fonts);
        let shaders = (sent.shaders != list.shaders).then(|| {
            list.shaders
                .iter()
                .map(|shader| (shader.guid, shader.source.to_string()))
                .collect()
        });
        if upload.is_none()
            && pages.is_empty()
            && sources.is_empty()
            && texts.is_empty()
            && videos.is_empty()
            && shaders.is_none()
            && sent.videos == list.videos
            && sent.vectors == list.vectors
            && sent.quads == list.quads
            && sent.masks == list.masks
            && sent.curves == list.curves
            && sent.mirrors == list.mirrors
            && sent.materials == list.materials
            && sent.params == list.params
        {
            return None;
        }
        sent.quads.clone_from(&list.quads);
        sent.masks.clone_from(&list.masks);
        sent.curves.clone_from(&list.curves);
        sent.mirrors.clone_from(&list.mirrors);
        sent.materials.clone_from(&list.materials);
        sent.params.clone_from(&list.params);
        sent.shaders.clone_from(&list.shaders);
        sent.vectors.clone_from(&list.vectors);
        sent.videos.clone_from(&list.videos);
        Some(UiFrame {
            quads: STANDARD.encode(bytemuck::cast_slice::<UiQuad, u8>(&list.quads)),
            masks: STANDARD.encode(bytemuck::cast_slice::<UiMask, u8>(&list.masks)),
            curves: STANDARD.encode(bytemuck::cast_slice::<UiCurve, u8>(&list.curves)),
            mirrors: STANDARD.encode(bytemuck::cast_slice::<UiMirror, u8>(&list.mirrors)),
            materials: list
                .materials
                .iter()
                .map(|m| [m.quad, m.shader, m.params[0], m.params[1]])
                .collect(),
            params: list
                .params
                .iter()
                .map(|param| (param.name.to_string(), param.value))
                .collect(),
            shaders,
            atlas_side: list.atlas_side,
            upload: upload.map(|upload| UiUpload {
                rect: upload.rect,
                texels: STANDARD.encode(&upload.texels),
            }),
            vector_side: list.vector_side,
            vectors: list
                .vectors
                .iter()
                .map(|v| (v.key, v.rect, v.source, v.crop))
                .collect(),
            vector_sources: sources
                .iter()
                .map(|source| (source.key, STANDARD.encode(&source.svg)))
                .collect(),
            vector_texts: texts.iter().map(WireText::from).collect(),
            vector_fonts: fonts
                .iter()
                .map(|font| (font.id, STANDARD.encode(&font.data)))
                .collect(),
            videos: list
                .videos
                .iter()
                .map(|video| (video.key, video.rect))
                .collect(),
            video_sources: videos
                .iter()
                .map(|video| (video.key, STANDARD.encode(&video.data)))
                .collect(),
            page_side: list.page_side,
            page_uploads: pages
                .iter()
                .map(|page| UiUpload {
                    rect: page.rect,
                    texels: STANDARD.encode(&page.texels),
                })
                .collect(),
        })
    });
    resources.insert(sent);
    frame
}

/// The editor's end. Hands what arrived into one list the renderer keeps, swapping buffers rather
/// than cloning them; glyph uploads queue there, so none needs a CPU copy of the atlas (#1898).
#[derive(Default)]
pub struct UiReceiver {
    quads: Vec<UiQuad>,
    masks: Vec<UiMask>,
    curves: Vec<UiCurve>,
    mirrors: Vec<UiMirror>,
    materials: Vec<UiMaterial>,
    params: Vec<UiParam>,
    shaders: Vec<UiShader>,
    side: u32,
    atlas_uploads: Vec<AtlasUpload>,
    /// Reused for every base64 decode of the quads and their tables.
    scratch: Vec<u8>,
    fresh: bool,
    vector_side: u32,
    vectors: Vec<UiVector>,
    /// Every file still resident, handed over each time: the renderer parses only keys it lacks,
    /// and a list it skipped loses nothing.
    vector_sources: Vec<UiVectorSource>,
    vector_texts: Vec<UiVectorText>,
    /// Every font received: a later text may use one sent long before.
    vector_fonts: Vec<UiFontSource>,
    videos: Vec<UiVideo>,
    /// Every video still shown, handed over each time as `vector_sources`.
    video_sources: Vec<UiVideoSource>,
    page_side: u32,
    /// Handed over once and not kept: the pages have no CPU copy (#1893).
    page_uploads: Vec<AtlasUpload>,
}

impl UiReceiver {
    pub fn receive(&mut self, frame: UiFrame) {
        let scratch = &mut self.scratch;
        let read = decode_into(&frame.quads, scratch, &mut self.quads)
            && decode_into(&frame.masks, scratch, &mut self.masks)
            && decode_into(&frame.curves, scratch, &mut self.curves)
            && decode_into(&frame.mirrors, scratch, &mut self.mirrors);
        if !read {
            tracing::warn!("the project sent UI quads this editor cannot read");
            self.quads.clear();
            return;
        }
        self.materials.clear();
        self.materials
            .extend(
                frame
                    .materials
                    .iter()
                    .map(|&[quad, shader, first, count]| UiMaterial {
                        quad,
                        shader,
                        params: [first, count],
                    }),
            );
        self.params.clear();
        self.params
            .extend(frame.params.into_iter().map(|(name, value)| UiParam {
                name: name.into(),
                value,
            }));
        if let Some(shaders) = frame.shaders {
            self.shaders = shaders
                .into_iter()
                .map(|(guid, source)| UiShader {
                    guid,
                    source: source.into(),
                })
                .collect();
        }
        self.fresh = true;
        self.page_side = frame.page_side;
        for page in &frame.page_uploads {
            match STANDARD.decode(&page.texels) {
                Ok(texels) => self.page_uploads.push(AtlasUpload {
                    rect: page.rect,
                    texels,
                }),
                Err(_) => tracing::warn!("the project sent a UI image this editor cannot read"),
            }
        }
        self.vector_side = frame.vector_side;
        self.vectors.clear();
        self.vectors.extend(
            frame
                .vectors
                .iter()
                .map(|&(key, rect, source, crop)| UiVector {
                    key,
                    rect,
                    source,
                    crop,
                }),
        );
        for (key, svg) in frame.vector_sources {
            match STANDARD.decode(&svg) {
                Ok(svg) => self.vector_sources.push(UiVectorSource {
                    key,
                    svg: svg.into(),
                }),
                Err(_) => tracing::warn!("the project sent a UI vector this editor cannot read"),
            }
        }
        self.vector_texts
            .extend(frame.vector_texts.into_iter().map(UiVectorText::from));
        for (id, data) in frame.vector_fonts {
            match STANDARD.decode(&data) {
                Ok(data) => self.vector_fonts.push(UiFontSource {
                    id,
                    data: data.into(),
                }),
                Err(_) => tracing::warn!("the project sent a UI font this editor cannot read"),
            }
        }
        let vectors = &self.vectors;
        let resident = |key: u64| vectors.iter().any(|vector| vector.source == key);
        self.vector_sources.retain(|source| resident(source.key));
        self.vector_texts.retain(|text| resident(text.key));
        self.videos.clear();
        self.videos.extend(
            frame
                .videos
                .iter()
                .map(|&(key, rect)| UiVideo { key, rect }),
        );
        for (key, data) in frame.video_sources {
            match STANDARD.decode(&data) {
                Ok(data) => self.video_sources.push(UiVideoSource {
                    key,
                    data: data.into(),
                }),
                Err(_) => tracing::warn!("the project sent a UI video this editor cannot read"),
            }
        }
        let videos = &self.videos;
        self.video_sources
            .retain(|source| videos.iter().any(|video| video.key == source.key));
        if frame.atlas_side != self.side {
            // Written for the old texture; a grown atlas is sent whole.
            self.side = frame.atlas_side;
            self.atlas_uploads.clear();
        }
        let Some(upload) = frame.upload else {
            return;
        };
        match checked(&upload, self.side) {
            Some(upload) => self.atlas_uploads.push(upload),
            None => {
                tracing::warn!(rect = ?upload.rect, "the project sent a glyph upload that does not fit")
            }
        }
    }

    /// Hands what arrived since the last call into `list`, the one the renderer keeps: what it has
    /// not taken yet stays queued there. False when nothing arrived.
    pub fn take_into(&mut self, list: &mut UiDrawList) -> bool {
        if !std::mem::take(&mut self.fresh) {
            return false;
        }
        // Each frame replaces these whole, so the receiver keeps the list's old buffers to refill.
        std::mem::swap(&mut self.quads, &mut list.quads);
        std::mem::swap(&mut self.masks, &mut list.masks);
        std::mem::swap(&mut self.curves, &mut list.curves);
        std::mem::swap(&mut self.mirrors, &mut list.mirrors);
        std::mem::swap(&mut self.materials, &mut list.materials);
        std::mem::swap(&mut self.params, &mut list.params);
        std::mem::swap(&mut self.vectors, &mut list.vectors);
        std::mem::swap(&mut self.videos, &mut list.videos);
        list.shaders.clone_from(&self.shaders);
        list.vector_sources.clone_from(&self.vector_sources);
        list.vector_texts.clone_from(&self.vector_texts);
        list.vector_fonts.clone_from(&self.vector_fonts);
        list.video_sources.clone_from(&self.video_sources);
        if list.atlas_side != self.side {
            list.atlas_uploads.clear();
        }
        list.atlas_side = self.side;
        list.atlas_uploads.append(&mut self.atlas_uploads);
        list.vector_side = self.vector_side;
        list.page_side = self.page_side;
        list.page_uploads.append(&mut self.page_uploads);
        true
    }
}

/// `upload` decoded, when it fits an atlas of `side`.
fn checked(upload: &UiUpload, side: u32) -> Option<AtlasUpload> {
    let [x0, y0, x1, y1] = upload.rect;
    if x0 >= x1 || y0 >= y1 || x1 > side || y1 > side {
        return None;
    }
    let texels = STANDARD.decode(&upload.texels).ok()?;
    (texels.len() == (x1 - x0) as usize * 4 * (y1 - y0) as usize).then_some(AtlasUpload {
        rect: upload.rect,
        texels,
    })
}

/// Decodes `encoded` into `out`, reusing both buffers. Read unaligned: bytes carry no alignment
/// for `f32`.
fn decode_into<T: bytemuck::Pod>(encoded: &str, scratch: &mut Vec<u8>, out: &mut Vec<T>) -> bool {
    scratch.clear();
    if STANDARD.decode_vec(encoded, scratch).is_err() || scratch.len() % size_of::<T>() != 0 {
        return false;
    }
    out.clear();
    out.extend(
        scratch
            .chunks_exact(size_of::<T>())
            .map(bytemuck::pod_read_unaligned::<T>),
    );
    true
}
