//! `data:` URI buffer decoding for glTF documents.
//!
//! A glTF spec-compliant data URI for a buffer looks like:
//!
//! ```text
//! data:application/octet-stream;base64,AAECAwQF…
//! ```
//!
//! Real-world variants seen in the wild include
//! `application/gltf-buffer` and the bare `data:;base64,…` shorthand
//! Blender emits when the glTF exporter inlines binary chunks. The
//! decoder accepts any media-type — the specifying detail is the
//! `;base64` parameter telling us how to interpret the payload.
//!
//! Non-base64 data URIs (`data:application/octet-stream,...`, payload
//! is percent-encoded raw bytes) are vanishingly rare for binary
//! buffers and are rejected: every real exporter emits base64.

use base64::Engine;

use super::GltfMeshError;

/// Decodes the payload of a `data:` URI (i.e., the substring **after**
/// the leading `data:` prefix). Splits the metadata header from the
/// payload at the first `,`, requires the `;base64` parameter, then
/// decodes with the RFC 4648 standard alphabet.
pub(super) fn decode(payload: &str) -> Result<Vec<u8>, GltfMeshError> {
    let (header, data) = payload
        .split_once(',')
        .ok_or(GltfMeshError::MalformedDataUri("missing `,` separator"))?;
    if !header_declares_base64(header) {
        return Err(GltfMeshError::MalformedDataUri(
            "only `;base64` payloads are supported",
        ));
    }
    base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|_| GltfMeshError::MalformedDataUri("base64 decode failed"))
}

/// Returns `true` when the metadata header (everything between
/// `data:` and the first `,`) ends with the `;base64` parameter — case
/// insensitive, parameter order ignored.
fn header_declares_base64(header: &str) -> bool {
    header
        .split(';')
        .skip(1) // First segment is the media-type.
        .any(|param| param.trim().eq_ignore_ascii_case("base64"))
}

#[cfg(test)]
mod tests;
