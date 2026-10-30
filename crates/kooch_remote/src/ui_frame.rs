//! The host's UI, carried to the editor in remote Play (#1691): the host lays it out but has no
//! renderer, so the editor's Game View draws it.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use kooch_core::resource::Resources;
use kooch_ecs::{AtlasUpload, UiDrawList, UiQuad};
use serde::{Deserialize, Serialize};

/// A [`UiDrawList`] on the wire. Bytes as base64: a quad is 96 bytes, and as JSON numbers it
/// would be three times that, every frame the UI changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiFrame {
    /// `UiQuad`s, back to front.
    pub quads: String,
    pub atlas_side: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<UiUpload>,
}

/// An [`AtlasUpload`] on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiUpload {
    pub rect: [u32; 4],
    pub texels: String,
}

/// The quads last sent, so a UI that did not change costs nothing on the wire.
#[derive(Default)]
struct SentUi(Vec<UiQuad>);

/// This frame's UI when it changed since the last reply. Takes the atlas upload: the editor keeps
/// what it was sent, so each texel crosses once.
pub(crate) fn take(resources: &mut Resources) -> Option<UiFrame> {
    let mut sent = resources.remove::<SentUi>().unwrap_or_default();
    let frame = resources.get_mut::<UiDrawList>().and_then(|list| {
        let upload = list.atlas_upload.take();
        if upload.is_none() && sent.0 == list.quads {
            return None;
        }
        sent.0.clone_from(&list.quads);
        Some(UiFrame {
            quads: STANDARD.encode(bytemuck::cast_slice::<UiQuad, u8>(&list.quads)),
            atlas_side: list.atlas_side,
            upload: upload.map(|upload| UiUpload {
                rect: upload.rect,
                texels: STANDARD.encode(&upload.texels),
            }),
        })
    });
    resources.insert(sent);
    frame
}

/// The editor's end. Keeps the atlas on the CPU, so frames that land between two draws merge
/// into one upload instead of the later one dropping the earlier one's glyphs.
#[derive(Default)]
pub struct UiReceiver {
    quads: Vec<UiQuad>,
    side: u32,
    atlas: Vec<u8>,
    dirty: Option<[u32; 4]>,
    fresh: bool,
}

impl UiReceiver {
    pub fn receive(&mut self, frame: UiFrame) {
        let Some(quads) = decode_quads(&frame.quads) else {
            tracing::warn!("the project sent UI quads this editor cannot read");
            return;
        };
        self.quads = quads;
        self.fresh = true;
        if frame.atlas_side != self.side {
            self.side = frame.atlas_side;
            self.atlas = vec![0; (self.side * self.side) as usize];
            self.dirty = None;
        }
        let Some(upload) = frame.upload else {
            return;
        };
        if !self.write(&upload) {
            tracing::warn!(rect = ?upload.rect, "the project sent a glyph upload that does not fit");
        }
    }

    /// What arrived since the last call. `pending` is an upload the renderer never took — a hidden
    /// Game View — folded in so its glyphs survive the replacement.
    pub fn take(&mut self, pending: Option<[u32; 4]>) -> Option<UiDrawList> {
        if !std::mem::take(&mut self.fresh) {
            return None;
        }
        let rect = match (self.dirty.take(), pending) {
            (Some(a), Some(b)) => Some(union(a, b)),
            (a, b) => a.or(b),
        }
        .filter(|rect| rect[2] <= self.side && rect[3] <= self.side);
        Some(UiDrawList {
            quads: self.quads.clone(),
            atlas_side: self.side,
            atlas_upload: rect.map(|rect| AtlasUpload {
                rect,
                texels: region(&self.atlas, self.side, rect),
            }),
        })
    }

    fn write(&mut self, upload: &UiUpload) -> bool {
        let [x0, y0, x1, y1] = upload.rect;
        if x0 >= x1 || y0 >= y1 || x1 > self.side || y1 > self.side {
            return false;
        }
        let Ok(texels) = STANDARD.decode(&upload.texels) else {
            return false;
        };
        let width = (x1 - x0) as usize;
        if texels.len() != width * (y1 - y0) as usize {
            return false;
        }
        for (row, y) in texels.chunks_exact(width).zip(y0..y1) {
            let start = (y * self.side + x0) as usize;
            self.atlas[start..start + width].copy_from_slice(row);
        }
        self.dirty = Some(
            self.dirty
                .map_or(upload.rect, |dirty| union(dirty, upload.rect)),
        );
        true
    }
}

fn decode_quads(encoded: &str) -> Option<Vec<UiQuad>> {
    let bytes = STANDARD.decode(encoded).ok()?;
    if bytes.len() % size_of::<UiQuad>() != 0 {
        return None;
    }
    // Collected rather than cast: a decoded `Vec<u8>` carries no alignment for `f32`.
    Some(bytemuck::pod_collect_to_vec(&bytes))
}

fn union(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn region(atlas: &[u8], side: u32, [x0, y0, x1, y1]: [u32; 4]) -> Vec<u8> {
    (y0..y1)
        .flat_map(|y| {
            let start = (y * side + x0) as usize;
            atlas[start..start + (x1 - x0) as usize].iter().copied()
        })
        .collect()
}
