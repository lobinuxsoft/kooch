use super::*;

#[test]
fn decodes_octet_stream_base64() {
    // `AAECAwQF` -> [0,1,2,3,4,5]
    let out = decode("application/octet-stream;base64,AAECAwQF").expect("standard payload decodes");
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
    let out =
        decode("application/gltf-buffer;base64,AAECAwQF").expect("gltf-buffer media type decodes");
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
