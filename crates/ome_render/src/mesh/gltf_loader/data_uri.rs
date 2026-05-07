//! `data:` URI buffer decoding for glTF documents.
//!
//! A glTF spec-compliant data URI for a buffer looks like:
//!
//! ```text
//! data:application/octet-stream;base64,AAECAwQF…
//! ```
//!
//! Real-world variants seen in the wild include
//! `application/gltf-buffer` and the bare `data:;base64,…` shorthand.
//! The base64 payload is RFC 4648 standard alphabet — `=` padding
//! optional in some emitters, so the decoder accepts both.
//!
//! The actual base64 implementation lands in the next commit (`#490`
//! split: hygiene + sidecar first, data URI second).

use super::GltfMeshError;

/// Decodes the payload of a `data:` URI (i.e., the substring **after**
/// the leading `data:` prefix). Currently a stub that surfaces a
/// distinct error so callers see "data URIs not yet supported"
/// instead of a generic missing-attribute message.
pub(super) fn decode(_payload: &str) -> Result<Vec<u8>, GltfMeshError> {
    Err(GltfMeshError::DataUriUnsupported)
}
