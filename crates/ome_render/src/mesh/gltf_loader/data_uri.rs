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
mod tests {
    use super::*;

    #[test]
    fn decodes_octet_stream_base64() {
        // `AAECAwQF` -> [0,1,2,3,4,5]
        let out =
            decode("application/octet-stream;base64,AAECAwQF").expect("standard payload decodes");
        assert_eq!(out, vec![0u8, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn decodes_blender_bare_form() {
        // Blender / many tools emit `data:;base64,...` (empty media type).
        let out = decode(";base64,AAECAwQF").expect("bare media type decodes");
        assert_eq!(out, vec![0u8, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn decodes_gltf_buffer_media_type() {
        let out = decode("application/gltf-buffer;base64,AAECAwQF")
            .expect("gltf-buffer media type decodes");
        assert_eq!(out, vec![0u8, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn rejects_payload_without_separator() {
        let err = decode("application/octet-stream;base64").unwrap_err();
        assert!(matches!(
            err,
            GltfMeshError::MalformedDataUri("missing `,` separator")
        ));
    }

    #[test]
    fn rejects_non_base64_payloads() {
        // Percent-encoded plain payload — not supported.
        let err = decode("application/octet-stream,Hello%20World").unwrap_err();
        assert!(matches!(
            err,
            GltfMeshError::MalformedDataUri("only `;base64` payloads are supported")
        ));
    }

    #[test]
    fn rejects_invalid_base64() {
        let err = decode("application/octet-stream;base64,!!!not_base64!!!").unwrap_err();
        assert!(matches!(
            err,
            GltfMeshError::MalformedDataUri("base64 decode failed")
        ));
    }
}
