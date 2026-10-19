//! Flat OpenEXR color channels named like beauty.R, which the image decoder does not select.
use std::collections::HashSet;

pub(crate) struct ColorGroup {
    pub width: u32,
    pub height: u32,
    pub alpha: bool,
    names: [String; 4],
}

/// Inspect headers only; the normal decoder remains the first choice for ordinary RGB channels.
pub(crate) fn metadata(bytes: &[u8]) -> Option<ColorGroup> {
    if !bytes.starts_with(&[0x76, 0x2f, 0x31, 0x01]) {
        return None;
    }
    let mut cursor = std::io::Cursor::new(bytes);
    let metadata = exr::meta::MetaData::read_from_buffered(&mut cursor, false).ok()?;
    if metadata.requirements.has_deep_data {
        return None;
    }
    // Each chunk has one u64 offset. Check the table before accepting header-only metadata.
    let offsets = metadata.headers.iter().try_fold(0u64, |count, header| count.checked_add(u64::try_from(header.chunk_count).ok()?))?;
    if cursor.position().checked_add(offsets.checked_mul(8)?)? > u64::try_from(bytes.len()).ok()? {
        return None;
    }
    for header in metadata.headers.iter().filter(|header| !header.deep) {
        let names: HashSet<String> = header.channels.list.iter().map(|channel| channel.name.to_string()).collect();
        for channel in &header.channels.list {
            let name = channel.name.to_string();
            let Some(prefix) = name.strip_suffix(".R") else { continue };
            let selected = ["R", "G", "B", "A"].map(|channel| format!("{prefix}.{channel}"));
            if !selected[..3].iter().all(|name| names.contains(name)) {
                continue;
            }
            let (width, height) = (u32::try_from(header.layer_size.0).ok()?, u32::try_from(header.layer_size.1).ok()?);
            let required = u64::from(width).checked_mul(u64::from(height))?.checked_mul(16)?;
            if width == 0 || height == 0 || required > image::Limits::default().max_alloc.unwrap_or(u64::MAX) {
                return None;
            }
            return Some(ColorGroup { width, height, alpha: names.contains(&selected[3]), names: selected });
        }
    }
    None
}

struct Pixels {
    width: usize,
    data: Option<Vec<f32>>,
}

/// Read only the chosen named color channels, preserving float HDR samples and optional alpha.
pub(crate) fn decode(bytes: &[u8]) -> Option<image::DynamicImage> {
    use exr::prelude::*;
    let group = metadata(bytes)?;
    let pixels = read()
        .no_deep_data()
        .largest_resolution_level()
        .specific_channels()
        .required(group.names[0].as_str())
        .required(group.names[1].as_str())
        .required(group.names[2].as_str())
        .optional(group.names[3].as_str(), 1.0f32)
        .collect_pixels(
            |size: Vec2<usize>, _| {
                let data = size.0.checked_mul(size.1).and_then(|n| n.checked_mul(4)).and_then(|n| {
                    let mut data = Vec::new();
                    data.try_reserve_exact(n).ok()?;
                    data.resize(n, 0.0);
                    Some(data)
                });
                Pixels { width: size.0, data }
            },
            |pixels: &mut Pixels, position: Vec2<usize>, (r, g, b, a): (f32, f32, f32, f32)| {
                if let Some(data) = &mut pixels.data {
                    let at = (position.1 * pixels.width + position.0) * 4;
                    if let Some(pixel) = data.get_mut(at..at + 4) {
                        pixel.copy_from_slice(&[r, g, b, a]);
                    }
                }
            },
        )
        .first_valid_layer()
        .all_attributes()
        .from_buffered(std::io::Cursor::new(bytes))
        .ok()?;
    let data = pixels.layer_data.channel_data.pixels.data?;
    Some(image::DynamicImage::ImageRgba32F(image::ImageBuffer::from_raw(group.width, group.height, data)?))
}
