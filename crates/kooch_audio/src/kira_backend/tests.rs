use super::*;

/// Minimal valid wav header + PCM frame. Kira's symphonia decoder
/// accepts wav out of the box (default feature flag).
fn minimal_wav_bytes() -> Vec<u8> {
    // Build a 16-bit mono PCM wav with two zero samples — enough
    // for symphonia to decode without errors.
    let sample_rate: u32 = 44100;
    let bits_per_sample: u16 = 16;
    let num_channels: u16 = 1;
    let samples: [i16; 2] = [0, 0];

    let byte_rate = sample_rate * (bits_per_sample as u32 / 8) * num_channels as u32;
    let block_align = (bits_per_sample / 8) * num_channels;
    let data_size = (samples.len() as u32) * (bits_per_sample as u32 / 8);
    let chunk_size = 36 + data_size;

    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&chunk_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&num_channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[test]
fn mock_backend_constructs() {
    let backend = KiraBackend::new_with_mock_backend().expect("mock backend should always succeed");
    assert_eq!(backend.sound_count(), 0);
    assert_eq!(backend.instance_count(), 0);
}

#[test]
fn load_minimal_wav_succeeds() {
    let mut backend = KiraBackend::new_with_mock_backend().unwrap();
    let bytes = minimal_wav_bytes();
    let handle = backend.load_sound(&bytes).expect("wav decode");
    assert!(backend.contains_sound(handle));
    assert_eq!(backend.sound_count(), 1);
}

#[test]
fn invalid_bytes_return_decode_error() {
    let mut backend = KiraBackend::new_with_mock_backend().unwrap();
    let err = backend.load_sound(b"not a real audio").unwrap_err();
    assert!(matches!(err, AudioError::Decode(_)));
}

#[test]
fn play_with_stale_handle_errs() {
    let mut backend = KiraBackend::new_with_mock_backend().unwrap();
    let h = backend.load_sound(&minimal_wav_bytes()).unwrap();
    backend.unload_sound(h);
    let err = backend.play(h, PlayParams::default()).unwrap_err();
    assert!(matches!(err, AudioError::SoundNotFound));
}

#[test]
fn play_returns_live_instance() {
    let mut backend = KiraBackend::new_with_mock_backend().unwrap();
    let h = backend.load_sound(&minimal_wav_bytes()).unwrap();
    let inst = backend.play(h, PlayParams::default()).unwrap();
    assert!(backend.contains_instance(inst));
    assert_eq!(backend.instance_count(), 1);
}

#[test]
fn listener_round_trips() {
    let mut backend = KiraBackend::new_with_mock_backend().unwrap();
    let pos = Vec3::new(5.0, 0.0, -3.0);
    let rot = Quat::from_rotation_x(0.5);
    backend.set_listener(pos, rot);
    let (got_pos, got_rot) = backend.listener();
    assert_eq!(got_pos, pos);
    assert!((got_rot.dot(rot)).abs() > 0.999);
}
