//! Buffer resolution for the glTF document: GLB inline blob today,
//! external URIs (sidecar `.bin`, `data:` URIs) tracked under #490.

use super::GltfMeshError;

/// Resolves the document's buffer bytes. GLB stores them inline (single
/// `blob`). External `.bin` sidecars and `data:` URI buffers are NOT
/// supported in PR-1 — return [`GltfMeshError::MissingAttribute`] so the
/// failure is observable. Production assets should ship as GLB anyway
/// (single file, no sidecar fragility).
pub(super) fn collect_buffers(
    document: &gltf::Document,
    glb_blob: Option<&[u8]>,
) -> Result<Vec<Vec<u8>>, GltfMeshError> {
    let mut out = Vec::with_capacity(document.buffers().len());
    for buffer in document.buffers() {
        match buffer.source() {
            gltf::buffer::Source::Bin => {
                let blob = glb_blob.ok_or(GltfMeshError::MissingAttribute("glb-binary-chunk"))?;
                out.push(blob.to_vec());
            }
            gltf::buffer::Source::Uri(_) => {
                return Err(GltfMeshError::MissingAttribute("external-uri"));
            }
        }
    }
    Ok(out)
}
