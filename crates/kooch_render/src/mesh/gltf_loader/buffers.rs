//! Buffer resolution for the glTF document.
//!
//! glTF documents reference their binary buffers in three flavours:
//!
//! 1. **GLB binary chunk** — packaged inline in the same file. The
//!    parser hands us the bytes via `Gltf::blob`.
//! 2. **External sidecar** — `"uri": "scene.bin"` (or any relative
//!    path). Lives next to the `.gltf` document on disk. This is what
//!    Blender's *glTF Separate* export emits.
//! 3. **Data URI** — `"uri": "data:application/octet-stream;base64,…"`
//!    embedded directly in the JSON. This is what Blender's
//!    *glTF Embedded* export emits, and what every standalone-`.gltf`
//!    publishing tool inlines. Decoded in [`super::data_uri`] (#490 follow-up).
//!
//! ## URI hygiene
//!
//! A `.gltf` document is parsed JSON authored on some other machine —
//! its `uri` field can claim anything, including paths that escape the
//! document's own directory. We resolve only relative single-component
//! paths and reject:
//!
//! - Absolute POSIX paths (`/foo`).
//! - Absolute Windows paths (`C:\…`, `\\server\…`).
//! - URI schemes other than `data:` (`file://`, `http://`, …).
//! - `..` path segments (traversal).
//!
//! Every commercial / open-source exporter (Blender, Maya, three.js,
//! gltfpack) emits sidecar URIs of the form `"name.bin"` — relative,
//! single component, no traversal. The reject list does not collide
//! with any legitimate workflow.

use std::path::Path;

use super::GltfMeshError;

/// Resolves the document's buffer bytes. Three sources covered:
///
/// - [`gltf::buffer::Source::Bin`] — GLB inline `blob` chunk.
/// - [`gltf::buffer::Source::Uri`] starting with `data:` — embedded
///   base64 (delegated to [`super::data_uri::decode`]).
/// - [`gltf::buffer::Source::Uri`] otherwise — sidecar path resolved
///   relative to `base_dir`. `base_dir` is the directory containing
///   the source `.gltf` document; when `None`, sidecar resolution
///   is impossible (e.g., bytes loaded from memory) and the load
///   fails with [`GltfMeshError::BufferUriUnresolvable`].
pub(super) fn collect_buffers(
    document: &gltf::Document,
    glb_blob: Option<&[u8]>,
    base_dir: Option<&Path>,
) -> Result<Vec<Vec<u8>>, GltfMeshError> {
    let mut out = Vec::with_capacity(document.buffers().len());
    for buffer in document.buffers() {
        match buffer.source() {
            gltf::buffer::Source::Bin => {
                let blob = glb_blob.ok_or(GltfMeshError::MissingAttribute("glb-binary-chunk"))?;
                out.push(blob.to_vec());
            }
            gltf::buffer::Source::Uri(uri) => {
                out.push(resolve_uri(uri, base_dir)?);
            }
        }
    }
    Ok(out)
}

/// Dispatches a `Uri` buffer source to the right resolver based on
/// shape — `data:` URIs decode in-process, everything else is treated
/// as a sidecar relative to `base_dir`.
fn resolve_uri(uri: &str, base_dir: Option<&Path>) -> Result<Vec<u8>, GltfMeshError> {
    if let Some(payload) = uri.strip_prefix("data:") {
        super::data_uri::decode(payload)
    } else {
        resolve_sidecar(uri, base_dir)
    }
}

/// Reads a sidecar buffer file. The URI must clear hygiene checks
/// ([`reject_unsafe_uri`]) and `base_dir` must be supplied (memory
/// loads have no anchor for relative resolution).
fn resolve_sidecar(uri: &str, base_dir: Option<&Path>) -> Result<Vec<u8>, GltfMeshError> {
    if let Some(reason) = reject_unsafe_uri(uri) {
        return Err(GltfMeshError::BufferUriRejected {
            uri: uri.to_string(),
            reason,
        });
    }
    let base = base_dir.ok_or(GltfMeshError::BufferUriUnresolvable)?;
    let resolved = base.join(uri);
    std::fs::read(&resolved).map_err(|source| GltfMeshError::BufferIo {
        uri: resolved.display().to_string(),
        source,
    })
}

/// Hygiene gate: returns `Some(reason)` if the URI must be rejected,
/// `None` if it is a safe relative path. Conservative by design — any
/// shape that doesn't look like a single-component relative file is
/// rejected, even if a particular exporter could in theory produce it.
fn reject_unsafe_uri(uri: &str) -> Option<&'static str> {
    if uri.is_empty() {
        return Some("empty uri");
    }
    // Absolute POSIX path or Windows UNC (`\\server\…`).
    if uri.starts_with('/') || uri.starts_with('\\') {
        return Some("absolute path");
    }
    // Windows drive letter (`C:foo`, `C:/foo`).
    let bytes = uri.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && (bytes[0] as char).is_ascii_alphabetic() {
        return Some("absolute path");
    }
    // Any URI scheme (`file://`, `http://`, `ftp://`, …). Sidecar
    // resolution only follows filesystem paths.
    if has_scheme(uri) {
        return Some("uri scheme not allowed");
    }
    // Path traversal via `..` segments.
    for component in uri.split(|c: char| c == '/' || c == '\\') {
        if component == ".." {
            return Some("`..` traversal not allowed");
        }
    }
    None
}

/// Returns `true` if `uri` looks like `scheme:rest` (RFC 3986 — at
/// least one alphanumeric / `+-.` byte before a `:` that is not a
/// drive-letter colon). Drive letters are filtered separately above.
fn has_scheme(uri: &str) -> bool {
    let Some(colon) = uri.find(':') else {
        return false;
    };
    if colon == 0 {
        return false;
    }
    let scheme = &uri[..colon];
    !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && scheme.len() > 1 // `C:` is a drive letter, not a scheme.
}

#[cfg(test)]
mod tests;
