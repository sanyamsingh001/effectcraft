//! Native generated float WAV import and PCM export precision.
#![cfg(not(target_arch = "wasm32"))]
use effectcraft_engine::Session;
use serde_json::json;
use std::path::{Path, PathBuf};

fn dir(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-out/export").join(name);
    std::fs::create_dir_all(&root).unwrap();
    root
}
fn float_wav(path: &Path) {
    let samples: Vec<u8> = (0..24_000).flat_map(|_| [0.25f32 + 2f32.powi(-25), -0.125f32 - 2f32.powi(-26)]).flat_map(f32::to_le_bytes).collect();
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + samples.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(3u16.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(48_000u32.to_le_bytes());
    bytes.extend((48_000u32 * 8).to_le_bytes());
    bytes.extend(8u16.to_le_bytes());
    bytes.extend(32u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((samples.len() as u32).to_le_bytes());
    bytes.extend(samples);
    std::fs::write(path, bytes).unwrap();
}
fn audio_session(path: &Path) -> Session {
    let mut s = effectcraft_host::session();
    s.execute("comp.new", json!({"name": "Float Audio", "width": 16, "height": 16, "duration": 0.5, "frameRate": 4})).unwrap();
    s.execute("file.import", json!({"paths": [path], "addToComp": true})).unwrap();
    s
}
fn comm_bits(path: &Path) -> u16 {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[8..12], b"AIFF");
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if &bytes[pos..pos + 4] == b"COMM" {
            assert_eq!(u16::from_be_bytes(bytes[pos + 8..pos + 10].try_into().unwrap()), 2);
            assert_eq!(u32::from_be_bytes(bytes[pos + 10..pos + 14].try_into().unwrap()), 24_000);
            return u16::from_be_bytes(bytes[pos + 14..pos + 16].try_into().unwrap());
        }
        pos += 8 + size + size % 2;
    }
    panic!("AIFF COMM chunk missing");
}
fn chunk<'a>(bytes: &'a [u8], name: &[u8; 4], big_endian: bool) -> &'a [u8] {
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let size: [u8; 4] = bytes[pos + 4..pos + 8].try_into().unwrap();
        let size = if big_endian { u32::from_be_bytes(size) } else { u32::from_le_bytes(size) } as usize;
        if &bytes[pos..pos + 4] == name {
            return &bytes[pos + 8..pos + 8 + size];
        }
        pos += 8 + size + size % 2;
    }
    panic!("missing PCM chunk");
}

#[test]
fn native_wav_float_and_aiff_integer_exports_match_their_reported_precision() {
    let dir = dir("aiff317-native-controls");
    let source = dir.join("float.wav");
    float_wav(&source);
    for (format, bits, reported) in [("wav", 32, "F32"), ("aiff", 24, "S24"), ("aiff", 16, "S16")] {
        let output = dir.join(format!("{format}-{bits}.{format}"));
        let mut session = audio_session(&source);
        session
            .execute("renderQueue.add", json!({"format":format,"output":output,"audio":"on","sampleRate":48_000,"audioChannels":"stereo","audioFormat":bits}))
            .unwrap();
        session.execute("renderQueue.render", json!({"wait":true})).unwrap();
        let queue = session.execute("renderQueue.list", json!({})).unwrap();
        assert_eq!(queue["items"][0]["status"], "Done");
        assert_eq!(queue["items"][0]["output"]["audio_format"], reported);
        let bytes = std::fs::read(&output).unwrap();
        if format == "wav" {
            let format = chunk(&bytes, b"fmt ", false);
            assert_eq!(u16::from_le_bytes(format[..2].try_into().unwrap()), 3);
            assert_eq!(u16::from_le_bytes(format[14..16].try_into().unwrap()), 32);
            let samples = chunk(&bytes, b"data", false);
            assert_eq!(samples.len(), 48_000 * 4);
            for stereo in samples.as_chunks::<8>().0 {
                assert_eq!(f32::from_le_bytes(stereo[..4].try_into().unwrap()), 0.25f32 + 2f32.powi(-25));
                assert_eq!(f32::from_le_bytes(stereo[4..].try_into().unwrap()), -0.125f32 - 2f32.powi(-26));
            }
        } else {
            assert_eq!(comm_bits(&output), bits);
            let samples = &chunk(&bytes, b"SSND", true)[8..];
            assert_eq!(samples.len(), 48_000 * usize::from(bits / 8));
        }
    }
}
