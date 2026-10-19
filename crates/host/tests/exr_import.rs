use effectcraft_engine::{
    project::{FootageKind, ItemId, ItemKind},
    time::Tick,
};
use serde_json::json;
use std::path::PathBuf;

struct FixtureDir(PathBuf);
impl Drop for FixtureDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn sequence(label: &str, prefixed: bool) -> (FixtureDir, PathBuf) {
    let dir = std::env::temp_dir().join("effectcraft-exr-sequences").join(format!("{}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (frame, color) in [[0.25, 0.0, 0.0, 1.0], [0.0, 0.5, 0.0, 1.0]].into_iter().enumerate() {
        use exr::prelude::*;
        let names = if prefixed { ["beauty.R", "beauty.G", "beauty.B", "beauty.A"] } else { ["R", "G", "B", "A"] };
        let channels = AnyChannels::sort(names.into_iter().zip(color).map(|(name, value)| AnyChannel::new(name, FlatSamples::F32(vec![value; 2]))).collect());
        let image = Image::from_layer(Layer::new((2, 1), LayerAttributes::default(), Encoding::FAST_LOSSLESS, channels));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write().to_buffered(&mut bytes).unwrap();
        let aux = effectcraft_media::exr_channels::read_exr_channels(bytes.get_ref()).unwrap();
        assert_eq!((aux.width, aux.height), (2, 1));
        for (name, value) in names.into_iter().zip(color) {
            assert_eq!(aux.get(name).unwrap(), &[value; 2], "pure-Rust reader validates the generated EXR");
        }
        std::fs::write(dir.join(format!("shot_{:04}.exr", frame + 1)), bytes.into_inner()).unwrap();
    }
    let first = dir.join("shot_0001.exr");
    (FixtureDir(dir), first)
}
fn assert_sequence_import(label: &str, prefixed: bool) {
    let (_dir, first) = sequence(label, prefixed);
    let mut session = effectcraft_host::session();
    let result = session.execute_checked("file.import", json!({"paths":[first]})).unwrap();
    eprintln!("EXR import {label}: {result}");
    assert_eq!(result["errors"], json!([]), "valid named-color EXR sequence should import");
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let id = ItemId(items[0].as_u64().unwrap());
    let ItemKind::Footage(footage) = &session.project.item(id).unwrap().kind else { panic!("expected footage") };
    assert_eq!(footage.kind, FootageKind::Sequence);
    assert_eq!(footage.sequence.len(), 2);
    assert_eq!((footage.width, footage.height), (2, 1));
    let pool = effectcraft_media::MediaPool::new();
    pool.set_read_ahead(false);
    let first = pool.frame_at(footage, Tick::ZERO).unwrap();
    let second = pool.frame_at(footage, footage.frame_rate.tick_of(1)).unwrap();
    eprintln!("decoded EXR frames: {:?} -> {:?}", first.data[0], second.data[0]);
    assert!(first.data.iter().all(|p| p[0] > 0.5 && p[1] == 0.0 && p[2] == 0.0 && p[3] == 1.0));
    assert!(second.data.iter().all(|p| p[0] == 0.0 && p[1] > 0.7 && p[2] == 0.0 && p[3] == 1.0));
}
#[test]
fn standard_rgba_exr_sequence_imports_and_decodes_in_order() {
    assert_sequence_import("rgba", false);
}
#[test]
fn named_rgba_exr_sequence_imports_and_decodes_in_order() {
    assert_sequence_import("named-rgba", true);
}

fn file_fixture(label: &str, bytes: &[u8]) -> (FixtureDir, PathBuf) {
    let dir = std::env::temp_dir().join(format!("effectcraft-exr-{}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("image.exr");
    std::fs::write(&path, bytes).unwrap();
    (FixtureDir(dir), path)
}
fn exr_bytes(names: &[&str], values: &[f32]) -> Vec<u8> {
    use exr::prelude::*;
    let channels = AnyChannels::sort(names.iter().zip(values).map(|(&name, &value)| AnyChannel::new(name, FlatSamples::F32(vec![value; 2]))).collect());
    let image = Image::from_layer(Layer::new((2, 1), LayerAttributes::default(), Encoding::FAST_LOSSLESS, channels));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write().to_buffered(&mut bytes).unwrap();
    bytes.into_inner()
}
fn assert_rejected(label: &str, bytes: &[u8]) {
    let (_dir, path) = file_fixture(label, bytes);
    let mut session = effectcraft_host::session();
    let result = session.execute_checked("file.import", json!({"paths":[path]})).unwrap();
    eprintln!("rejected EXR {label}: {result}");
    assert!(result["items"].as_array().unwrap().is_empty());
    assert_eq!(result["errors"].as_array().unwrap().len(), 1);
}
#[test]
fn malformed_exr_is_rejected() {
    assert_rejected("malformed", &[0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0, 0]);
}
#[test]
fn auxiliary_only_exr_remains_rejected_as_color_footage() {
    let bytes = exr_bytes(&["Z"], &[100.0]);
    let aux = effectcraft_media::exr_channels::read_exr_channels(&bytes).unwrap();
    assert_eq!(aux.get("Z").unwrap(), &[100.0; 2]);
    assert_rejected("no-color", &bytes);
}
#[test]
fn deep_exr_header_is_rejected_before_pixel_decoding() {
    use std::io::Write;
    let flat = exr_bytes(&["beauty.R", "beauty.G", "beauty.B"], &[0.5, 0.25, 0.0]);
    let mut metadata = exr::meta::MetaData::read_from_buffered(&mut std::io::Cursor::new(&flat), false).unwrap();
    metadata.headers[0].deep = true;
    metadata.headers[0].deep_data_version = Some(1);
    metadata.headers[0].max_samples_per_pixel = Some(1);
    metadata.headers[0].own_attributes.layer_name = Some(exr::prelude::Text::from("deep"));
    metadata.headers[0]
        .own_attributes
        .other
        .insert(exr::prelude::Text::from("type"), exr::prelude::AttributeValue::BlockType(exr::meta::attribute::BlockType::DeepScanLine));
    let mut header_only = vec![0x76, 0x2f, 0x31, 0x01];
    // OpenEXR version 2 with deep and multipart flags, serialized through the public header API.
    header_only.write_all(&(2u32 | (1 << 11) | (1 << 12)).to_le_bytes()).unwrap();
    exr::meta::header::Header::write_all(&metadata.headers, &mut header_only, true).unwrap();
    let parsed = exr::meta::MetaData::read_from_buffered(&mut std::io::Cursor::new(&header_only), false).unwrap();
    assert!(parsed.headers[0].deep, "the fixture has an independently parsed deep header");
    assert_rejected("deep-header", &header_only);
}
fn assert_float_import(label: &str, prefix: bool, alpha: bool) {
    let names = if prefix { ["beauty.R", "beauty.G", "beauty.B", "beauty.A"] } else { ["R", "G", "B", "A"] };
    let values = if alpha { [2.0, 1.0, 0.5, 0.5] } else { [4.0, 1.0, 0.0, 1.0] };
    let channels = if alpha { &names[..] } else { &names[..3] };
    let bytes = exr_bytes(channels, &values[..channels.len()]);
    let (_dir, path) = file_fixture(label, &bytes);
    let mut session = effectcraft_host::session();
    let result = session.execute_checked("file.import", json!({"paths":[path]})).unwrap();
    eprintln!("float EXR {label}: {result}");
    assert_eq!(result["errors"], json!([]));
    let ItemKind::Footage(footage) = &session.project.item(ItemId(result["items"][0].as_u64().unwrap())).unwrap().kind else { panic!("expected footage") };
    let image = effectcraft_media::MediaPool::new().frame_at(footage, Tick::ZERO).unwrap();
    let pixel = image.data[0];
    eprintln!("float EXR pixel {label}: {pixel:?}");
    if alpha {
        assert_eq!(footage.alpha, effectcraft_engine::project::AlphaMode::Premultiplied);
        assert!(pixel[0] > 0.85 && pixel[0] < 1.0 && pixel[3] == 0.5, "premultiplied HDR channels retain their alpha and values");
    } else {
        assert_eq!(footage.alpha, effectcraft_engine::project::AlphaMode::Ignore);
        assert!(pixel[0] > 1.8 && (pixel[1] - 1.0).abs() < 1e-5 && pixel[3] == 1.0, "opaque HDR channels retain values above one");
    }
}
#[test]
fn standard_float_rgba_retains_hdr_and_premultiplied_alpha() {
    assert_float_import("standard-hdr", false, true);
}
#[test]
fn named_rgb_without_alpha_imports_as_opaque_hdr() {
    assert_float_import("named-rgb", true, false);
}
#[test]
fn named_float_rgba_retains_hdr_and_premultiplied_alpha() {
    assert_float_import("named-hdr", true, true);
}

#[test]
fn standard_rgb_without_alpha_retains_opaque_hdr() {
    assert_float_import("standard-rgb", false, false);
}
#[test]
fn oversized_named_color_header_is_rejected_before_pixels_are_allocated() {
    let flat = exr_bytes(&["beauty.R", "beauty.G", "beauty.B"], &[0.5, 0.25, 0.0]);
    let mut metadata = exr::meta::MetaData::read_from_buffered(&mut std::io::Cursor::new(&flat), false).unwrap();
    metadata.headers[0].layer_size = exr::prelude::Vec2(8193, 4096);
    metadata.headers[0].chunk_count = metadata.headers[0].blocks_increasing_y_order().len();
    let mut header_only = flat[..8].to_vec();
    exr::meta::header::Header::write_all(&metadata.headers, &mut header_only, false).unwrap();
    let parsed = exr::meta::MetaData::read_from_buffered(&mut std::io::Cursor::new(&header_only), false).unwrap();
    assert_eq!(parsed.headers[0].layer_size, exr::prelude::Vec2(8193, 4096));
    exr::meta::MetaData::validate(&parsed.headers, false).unwrap();
    let offsets = parsed.headers.iter().map(|header| header.chunk_count).sum::<usize>();
    header_only.resize(header_only.len() + offsets * 8, 0);
    assert_rejected("oversized-header", &header_only);
}

#[test]
fn truncated_named_color_offset_table_is_rejected() {
    let flat = exr_bytes(&["beauty.R", "beauty.G", "beauty.B"], &[0.5, 0.25, 0.0]);
    let mut cursor = std::io::Cursor::new(&flat);
    let metadata = exr::meta::MetaData::read_from_buffered(&mut cursor, false).unwrap();
    exr::meta::MetaData::validate(&metadata.headers, false).unwrap();
    let header_only = &flat[..cursor.position() as usize];
    assert!(metadata.headers.iter().any(|header| header.chunk_count > 0));
    assert_rejected("truncated-offsets", header_only);
}
