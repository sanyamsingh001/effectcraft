//! glTF 2.0 import, written from the Khronos glTF 2.0 specification.
//!
//! Supports `.gltf` (JSON with external `.bin`/image files or `data:` URIs) and `.glb` (binary
//! container: a JSON chunk and a BIN chunk). Imports the default scene's node hierarchy (TRS or
//! matrix), triangle primitives (`TRIANGLES`, `TRIANGLE_STRIP`, `TRIANGLE_FAN`) with POSITION,
//! NORMAL, TANGENT, TEXCOORD_0, JOINTS_0 and WEIGHTS_0, PBR metallic-roughness materials
//! (base colour, metallic-roughness, normal, occlusion and emissive textures; alpha modes;
//! double-sided; `KHR_materials_emissive_strength`, `KHR_materials_unlit`), embedded or
//! external PNG/JPEG images with sampler wrap modes, skins and animations (step, linear and
//! cubic-spline translation/rotation/scale), perspective and orthographic cameras, and
//! `KHR_lights_punctual` lights (directional, point, spot). Sparse accessors and morph targets
//! are ignored. Owned resource buffers and decoded allocations are limited to 128 MiB each
//! and 256 MiB per import, including repeated accessor reads, derived geometry, encoded image
//! copies, decoded textures and scratch buffers; larger models return an unsupported-data error.
//! The caller's borrowed input and parser metadata are outside this resource-data budget.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::Cursor;

use image::ImageDecoder;
use std::sync::Arc;

use serde_json::Value;

use crate::{
    AlphaMode, Animation, Channel, ChannelPath, Interp, Mat4, Material, Mesh, Model, ModelError, Node, Primitive, Result, Skin, TexRef, Texture, Wrap, perr,
};

const GLB_MAGIC: u32 = 0x4654_6C67;
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

// Bound owned resource data and decoded geometry independently of claimed counts. The
// total includes retained sources, repeated accessors, sparse zeros and derived vertex data.
const MAX_ACCESSOR_BYTES: usize = 128 * 1024 * 1024;
const MAX_DECODED_ACCESSOR_BYTES: usize = 256 * 1024 * 1024;

/// Split a GLB container into its JSON text and BIN chunk.
pub fn split_glb(b: &[u8]) -> Result<(&[u8], Option<&[u8]>)> {
    let u32_at = |o: usize| -> Option<u32> { o.checked_add(4).and_then(|end| b.get(o..end)).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])) };
    if u32_at(0) != Some(GLB_MAGIC) {
        return Err(perr("not a GLB file"));
    }
    if u32_at(4) != Some(2) {
        return Err(ModelError::Unsupported(format!("GLB version {}", u32_at(4).unwrap_or(0))));
    }
    let total = (u32_at(8).unwrap_or(0) as usize).min(b.len());
    let mut o = 12usize;
    let mut json = None;
    let mut bin = None;
    while o.checked_add(8).is_some_and(|end| end <= total) {
        let len = u32_at(o).unwrap_or(0) as usize;
        let ty = u32_at(o + 4).unwrap_or(0);
        let start = o.checked_add(8).ok_or_else(|| perr("GLB chunk offset overflow"))?;
        let end = start.checked_add(len).filter(|end| *end <= total).ok_or_else(|| perr("GLB chunk runs past the end of the file"))?;
        let data = b.get(start..end).ok_or_else(|| perr("GLB chunk runs past the end of the file"))?;
        match ty {
            CHUNK_JSON if json.is_none() => json = Some(data),
            CHUNK_BIN if bin.is_none() => bin = Some(data),
            _ => {}
        }
        o = end;
    }
    Ok((json.ok_or_else(|| perr("GLB without a JSON chunk"))?, bin))
}

/// Decode standard base64 (with or without padding; whitespace ignored).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let len = base64_len(s)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len).ok()?;
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// Count decoded bytes before allocating. Whitespace must not increase the output capacity.
fn base64_len(s: &str) -> Option<usize> {
    let mut count = 0usize;
    for c in s.bytes() {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'-' | b'/' | b'_' => count += 1,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => {}
            _ => return None,
        }
    }
    Some(count / 4 * 3 + count % 4 * 6 / 8)
}

/// Percent-decode a relative URI.
pub fn uri_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    percent_bytes(s, |byte| out.push(byte));
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_bytes(s: &str, mut emit: impl FnMut(u8)) {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            emit(v);
            i += 3;
        } else {
            emit(b[i]);
            i += 1;
        }
    }
}

fn lossy_utf8_parts(mut bytes: &[u8], mut emit: impl FnMut(&str)) {
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                emit(text);
                return;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                emit(std::str::from_utf8(&bytes[..valid]).unwrap_or(""));
                emit("\u{FFFD}");
                bytes = &bytes[valid + error.error_len().unwrap_or(bytes.len() - valid)..];
            }
        }
    }
}

struct Doc<'a> {
    j: &'a Value,
    buffers: Vec<Vec<u8>>,
    decoded_remaining: Cell<usize>,
}

fn arr<'v>(v: &'v Value, k: &str) -> &'v [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}
fn idx(v: &Value, k: &str) -> Option<usize> {
    v.get(k).and_then(Value::as_u64).map(|x| x as usize)
}
fn num(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(d)
}
fn nums<const N: usize>(v: &Value, k: &str, d: [f64; N]) -> [f64; N] {
    let mut out = d;
    if let Some(a) = v.get(k).and_then(Value::as_array) {
        for (i, x) in a.iter().take(N).enumerate() {
            out[i] = x.as_f64().unwrap_or(d[i]);
        }
    }
    out
}

fn size(v: &Value, key: &str, default: usize) -> Result<usize> {
    let Some(value) = v.get(key) else { return Ok(default) };
    let number = value.as_u64().ok_or_else(|| perr(format!("{key} must be a non-negative integer")))?;
    usize::try_from(number).map_err(|_| perr(format!("{key} is too large for this platform")))
}

fn zeroed<T: Default + Clone>(len: usize) -> Result<Vec<T>> {
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| perr("not enough memory for model accessor"))?;
    out.resize(len, T::default());
    Ok(out)
}

impl Doc<'_> {
    fn reserve_bytes(&self, bytes: usize, kind: &str) -> Result<()> {
        if bytes > MAX_ACCESSOR_BYTES {
            return Err(ModelError::Unsupported(format!("{kind} exceeds {MAX_ACCESSOR_BYTES} allocated resource bytes")));
        }
        let remaining = self
            .decoded_remaining
            .get()
            .checked_sub(bytes)
            .ok_or_else(|| ModelError::Unsupported(format!("model exceeds {MAX_DECODED_ACCESSOR_BYTES} allocated resource bytes")))?;
        self.decoded_remaining.set(remaining);
        Ok(())
    }

    /// Charge an owned resource once. External resolver allocation is outside the parser;
    /// its retained capacity is checked before any subsequent model allocation.
    fn load_uri(&self, uri: &str, resolve: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Vec<u8>> {
        if let Some(rest) = uri.strip_prefix("data:") {
            let (meta, data) = rest.split_once(',').ok_or_else(|| perr("malformed data URI"))?;
            if meta.ends_with(";base64") {
                let len = base64_len(data).ok_or_else(|| perr("bad base64 in data URI"))?;
                self.reserve_bytes(len, "resource buffer")?;
                return base64_decode(data).ok_or_else(|| perr("bad base64 in data URI"));
            }
            let mut len = 0;
            percent_bytes(data, |_| len += 1);
            self.reserve_bytes(len, "resource buffer")?;
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(len).map_err(|_| perr("not enough memory for model resource"))?;
            percent_bytes(data, |byte| bytes.push(byte));
            if std::str::from_utf8(&bytes).is_ok() {
                return Ok(bytes);
            }
            // Preserve the existing lossy UTF-8 data-URI semantics. Only malformed UTF-8
            // requires another allocation; charge that copy before creating replacements.
            let mut len = 0;
            lossy_utf8_parts(&bytes, |part| len += part.len());
            self.reserve_bytes(len, "resource UTF-8 conversion")?;
            let mut out = Vec::new();
            out.try_reserve_exact(len).map_err(|_| perr("not enough memory for model resource"))?;
            lossy_utf8_parts(&bytes, |part| out.extend_from_slice(part.as_bytes()));
            return Ok(out);
        }
        let name = uri_decode(uri);
        let data = resolve(&name).ok_or(ModelError::Missing(name))?;
        self.reserve_bytes(data.capacity(), "resource buffer")?;
        Ok(data)
    }

    fn accessor_len(&self, count: usize, components: usize) -> Result<usize> {
        let len = count.checked_mul(components).ok_or_else(|| perr("accessor component count overflow"))?;
        let bytes = len.checked_mul(std::mem::size_of::<u32>()).ok_or_else(|| perr("accessor storage size overflow"))?;
        self.reserve_bytes(bytes, "accessor")?;
        Ok(len)
    }

    /// Reserve derived geometry before copying or expanding any accessor data.
    fn geometry_vec<T>(&self, count: usize) -> Result<Vec<T>> {
        let bytes = count.checked_mul(std::mem::size_of::<T>()).ok_or_else(|| perr("geometry storage size overflow"))?;
        self.reserve_bytes(bytes, "geometry buffer")?;
        let mut out = Vec::new();
        out.try_reserve_exact(count).map_err(|_| perr("not enough memory for model geometry"))?;
        Ok(out)
    }

    fn float_rows<const N: usize>(&self, accessor: Option<usize>) -> Result<Vec<[f32; N]>> {
        let Some(accessor) = accessor else { return Ok(Vec::new()) };
        let (values, _) = self.floats(accessor)?;
        let rows = values.as_chunks::<N>().0;
        let mut out = self.geometry_vec(rows.len())?;
        out.extend(rows.iter().copied());
        Ok(out)
    }

    /// Bound native decode, float conversion and final RGBA storage before decoding pixels.
    fn texture(&self, bytes: &[u8]) -> Result<Texture> {
        let image_error = |error: image::ImageError| match error {
            image::ImageError::Limits(_) => ModelError::Unsupported("model image exceeds decoder limits".into()),
            other => perr(format!("image: {other}")),
        };
        let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| perr(format!("image: {e}")))?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(MAX_ACCESSOR_BYTES as u64);
        reader.limits(limits);
        let decoder = reader.into_decoder().map_err(image_error)?;
        let (width, height) = decoder.dimensions();
        let count =
            usize::try_from(u64::from(width) * u64::from(height)).map_err(|_| ModelError::Unsupported("model image dimensions exceed this platform".into()))?;
        let bytes = count.checked_mul(std::mem::size_of::<[f32; 4]>()).ok_or_else(|| ModelError::Unsupported("model image storage size overflows".into()))?;
        let native = usize::try_from(decoder.total_bytes()).map_err(|_| ModelError::Unsupported("model image decode size overflows".into()))?;
        self.reserve_bytes(native, "image decoder")?;
        self.reserve_bytes(bytes, "image conversion")?;
        let mut data = self.geometry_vec(count)?;
        let image = image::DynamicImage::from_decoder(decoder).map_err(image_error)?.into_rgba32f();
        data.extend(image.pixels().map(|p| p.0));
        Ok(Texture { width, height, data, wrap: [Wrap::Repeat; 2] })
    }

    fn wrapped_texture(&self, image: &Texture, wrap: [Wrap; 2]) -> Result<Texture> {
        let mut data = self.geometry_vec(image.data.len())?;
        data.extend_from_slice(&image.data);
        Ok(Texture { width: image.width, height: image.height, data, wrap })
    }

    fn view_bytes(&self, bv: usize) -> Result<&[u8]> {
        let view = arr(self.j, "bufferViews").get(bv).ok_or_else(|| perr(format!("no bufferView {bv}")))?;
        let buf = self.buffers.get(idx(view, "buffer").unwrap_or(0)).ok_or_else(|| perr("bufferView buffer out of range"))?;
        let start = size(view, "byteOffset", 0)?;
        let end = start.checked_add(size(view, "byteLength", 0)?).ok_or_else(|| perr("bufferView byte range overflow"))?;
        buf.get(start..end).ok_or_else(|| perr("bufferView reads past its buffer"))
    }

    /// Validate the entire accessor range before reserving any decoded storage.
    fn accessor_bytes(&self, acc: &Value, bv: usize, count: usize, elem: usize) -> Result<(&[u8], usize, usize)> {
        let view = arr(self.j, "bufferViews").get(bv).ok_or_else(|| perr(format!("no bufferView {bv}")))?;
        let bytes = self.view_bytes(bv)?;
        let base = size(acc, "byteOffset", 0)?;
        let stride = size(view, "byteStride", elem)?;
        let stride = if stride == 0 { elem } else { stride };
        if stride < elem {
            return Err(perr("accessor byte stride is smaller than its element"));
        }
        let len = if count == 0 {
            0
        } else {
            (count - 1).checked_mul(stride).and_then(|last| last.checked_add(elem)).ok_or_else(|| perr("accessor byte range overflow"))?
        };
        let end = base.checked_add(len).ok_or_else(|| perr("accessor byte range overflow"))?;
        if end > bytes.len() {
            return Err(perr("accessor reads past its bufferView"));
        }
        Ok((bytes, base, stride))
    }

    /// An accessor as `f32` rows of `n` components (normalised integers mapped to 0..1 / -1..1).
    fn floats(&self, a: usize) -> Result<(Vec<f32>, usize)> {
        let acc = arr(self.j, "accessors").get(a).ok_or_else(|| perr(format!("no accessor {a}")))?;
        let n = match acc.get("type").and_then(Value::as_str).unwrap_or("") {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT2" => 4,
            "MAT3" => 9,
            "MAT4" => 16,
            t => return Err(perr(format!("accessor type `{t}`"))),
        };
        let count = size(acc, "count", 0)?;
        let ct = idx(acc, "componentType").unwrap_or(5126);
        let norm = acc.get("normalized").and_then(Value::as_bool).unwrap_or(false);
        let csize = match ct {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(perr(format!("componentType {ct}"))),
        };
        let len = self.accessor_len(count, n)?;
        let Some(bv) = idx(acc, "bufferView") else {
            // No buffer view: all zeros (sparse substitution is not supported).
            return Ok((zeroed(len)?, n));
        };
        // Matrices of 1/2-byte components pad columns to 4 bytes; ignored (rare).
        let (buf, base, stride) = self.accessor_bytes(acc, bv, count, csize * n)?;
        let mut out: Vec<f32> = zeroed(len)?;
        for (i, row) in out.chunks_exact_mut(n).enumerate() {
            let start = i.checked_mul(stride).and_then(|offset| base.checked_add(offset)).ok_or_else(|| perr("accessor byte offset overflow"))?;
            for (c, value) in row.iter_mut().enumerate() {
                let o = start.checked_add(c * csize).ok_or_else(|| perr("accessor byte offset overflow"))?;
                let end = o.checked_add(csize).ok_or_else(|| perr("accessor byte offset overflow"))?;
                let s = buf.get(o..end).ok_or_else(|| perr("accessor reads past its buffer"))?;
                *value = match ct {
                    5126 => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                    5125 => u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32,
                    5123 => {
                        let v = u16::from_le_bytes([s[0], s[1]]) as f32;
                        if norm { v / 65535.0 } else { v }
                    }
                    5122 => {
                        let v = i16::from_le_bytes([s[0], s[1]]) as f32;
                        if norm { (v / 32767.0).max(-1.0) } else { v }
                    }
                    5121 => {
                        let v = s[0] as f32;
                        if norm { v / 255.0 } else { v }
                    }
                    _ => {
                        let v = s[0] as i8 as f32;
                        if norm { (v / 127.0).max(-1.0) } else { v }
                    }
                };
            }
        }
        Ok((out, n))
    }

    /// Integer accessor (indices, joints) read exactly.
    fn ints(&self, a: usize) -> Result<Vec<u32>> {
        let acc = arr(self.j, "accessors").get(a).ok_or_else(|| perr(format!("no accessor {a}")))?;
        let ct = idx(acc, "componentType").unwrap_or(5125);
        if ct == 5126 {
            return Ok(self.floats(a)?.0.into_iter().map(|v| v as u32).collect());
        }
        let n = match acc.get("type").and_then(Value::as_str).unwrap_or("SCALAR") {
            "VEC4" => 4,
            "VEC3" => 3,
            "VEC2" => 2,
            _ => 1,
        };
        let count = size(acc, "count", 0)?;
        let csize = match ct {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            _ => 4,
        };
        let len = self.accessor_len(count, n)?;
        let Some(bv) = idx(acc, "bufferView") else { return zeroed(len) };
        let (buf, base, stride) = self.accessor_bytes(acc, bv, count, csize * n)?;
        let mut out = Vec::new();
        out.try_reserve_exact(len).map_err(|_| perr("not enough memory for model accessor"))?;
        for i in 0..count {
            let start = i.checked_mul(stride).and_then(|offset| base.checked_add(offset)).ok_or_else(|| perr("accessor byte offset overflow"))?;
            for c in 0..n {
                let o = start.checked_add(c * csize).ok_or_else(|| perr("accessor byte offset overflow"))?;
                let end = o.checked_add(csize).ok_or_else(|| perr("accessor byte offset overflow"))?;
                let s = buf.get(o..end).ok_or_else(|| perr("accessor reads past its buffer"))?;
                out.push(match csize {
                    1 => s[0] as u32,
                    2 => u16::from_le_bytes([s[0], s[1]]) as u32,
                    _ => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                });
            }
        }
        Ok(out)
    }
}

/// Parse a `.gltf` (JSON) or `.glb` file.
pub fn parse(bytes: &[u8], resolve: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Model> {
    parse_with_budget(bytes, resolve, MAX_DECODED_ACCESSOR_BYTES)
}

fn parse_with_budget(bytes: &[u8], resolve: &dyn Fn(&str) -> Option<Vec<u8>>, budget: usize) -> Result<Model> {
    let (json, bin) = if bytes.starts_with(b"glTF") { split_glb(bytes)? } else { (bytes, None) };
    let j: Value = serde_json::from_slice(json).map_err(|e| perr(format!("glTF JSON: {e}")))?;
    let ver = j.get("asset").and_then(|a| a.get("version")).and_then(Value::as_str).unwrap_or("");
    if !ver.starts_with('2') {
        return Err(ModelError::Unsupported(format!("glTF version `{ver}` (2.0 required)")));
    }
    for ext in arr(&j, "extensionsRequired") {
        let e = ext.as_str().unwrap_or("");
        if !matches!(e, "KHR_materials_unlit" | "KHR_materials_emissive_strength") {
            return Err(ModelError::Unsupported(format!("required extension {e}")));
        }
    }
    let mut d = Doc { j: &j, buffers: vec![], decoded_remaining: Cell::new(budget) };
    for (i, b) in arr(&j, "buffers").iter().enumerate() {
        let data = match b.get("uri").and_then(Value::as_str) {
            Some(u) => d.load_uri(u, resolve)?,
            None if i == 0 => {
                let bin = bin.ok_or_else(|| perr("buffer 0 has no uri and the file has no BIN chunk"))?;
                d.reserve_bytes(bin.len(), "resource buffer")?;
                bin.to_vec()
            }
            None => vec![],
        };
        d.buffers.push(data);
    }

    // Images → textures (decoded once per image; samplers set the wrap modes).
    let mut images: Vec<Option<Arc<Texture>>> = vec![];
    for im in arr(&j, "images") {
        let data = if let Some(u) = im.get("uri").and_then(Value::as_str) {
            match d.load_uri(u, resolve) {
                Ok(data) => Some(data),
                Err(error @ ModelError::Unsupported(_)) => return Err(error),
                _ => None,
            }
        } else if let Some(bv) = idx(im, "bufferView") {
            let bytes = d.view_bytes(bv)?;
            d.reserve_bytes(bytes.len(), "encoded image copy")?;
            Some(bytes.to_vec())
        } else {
            None
        };
        let texture = match data.as_deref().map(|bytes| d.texture(bytes)) {
            Some(Ok(texture)) => Some(Arc::new(texture)),
            Some(Err(error @ ModelError::Unsupported(_))) => return Err(error),
            // Keep the existing missing/malformed-image fallback; oversized data is an error.
            _ => None,
        };
        images.push(texture);
    }
    let wrap = |v: u64| match v {
        33071 => Wrap::Clamp,
        33648 => Wrap::Mirror,
        _ => Wrap::Repeat,
    };
    let mut textures: Vec<Arc<Texture>> = vec![];
    let mut tex_map: Vec<Option<usize>> = vec![];
    let mut variants: HashMap<(usize, [u8; 2]), Arc<Texture>> = HashMap::new();
    for t in arr(&j, "textures") {
        let Some(img) = idx(t, "source").and_then(|s| images.get(s).cloned().flatten()) else {
            tex_map.push(None);
            continue;
        };
        let samp = idx(t, "sampler").and_then(|s| arr(&j, "samplers").get(s));
        let ws = samp.and_then(|s| s.get("wrapS")).and_then(Value::as_u64).unwrap_or(10497);
        let wt = samp.and_then(|s| s.get("wrapT")).and_then(Value::as_u64).unwrap_or(10497);
        let w = [wrap(ws), wrap(wt)];
        // Identical image/sampler combinations share pixel storage. Different wrapping still
        // uses an independently bounded copy because Texture owns its pixel vector.
        let key = (Arc::as_ptr(&img) as usize, w.map(|mode| mode as u8));
        let tex = if let Some(texture) = variants.get(&key) {
            texture.clone()
        } else {
            let texture = if img.wrap == w { img } else { Arc::new(d.wrapped_texture(&img, w)?) };
            variants.insert(key, texture.clone());
            texture
        };
        tex_map.push(Some(textures.len()));
        textures.push(tex);
    }
    let tref = |v: Option<&Value>| -> Option<TexRef> {
        let v = v?;
        let t = tex_map.get(idx(v, "index")?).copied().flatten()?;
        Some(TexRef { texture: t, uv_set: idx(v, "texCoord").unwrap_or(0) as u32 })
    };

    let mut materials = vec![];
    for m in arr(&j, "materials") {
        let pbr = m.get("pbrMetallicRoughness").cloned().unwrap_or(Value::Null);
        let bc = nums(&pbr, "baseColorFactor", [1.0; 4]);
        let ext = m.get("extensions");
        let strength = ext.and_then(|e| e.get("KHR_materials_emissive_strength")).map_or(1.0, |e| num(e, "emissiveStrength", 1.0));
        let em = nums(m, "emissiveFactor", [0.0; 3]);
        materials.push(Material {
            name: m.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            base_color: bc.map(|v| v as f32),
            base_color_tex: tref(pbr.get("baseColorTexture")),
            metallic: num(&pbr, "metallicFactor", 1.0) as f32,
            roughness: num(&pbr, "roughnessFactor", 1.0) as f32,
            metallic_roughness_tex: tref(pbr.get("metallicRoughnessTexture")),
            normal_tex: tref(m.get("normalTexture")),
            normal_scale: m.get("normalTexture").map_or(1.0, |n| num(n, "scale", 1.0)) as f32,
            occlusion_tex: tref(m.get("occlusionTexture")),
            occlusion_strength: m.get("occlusionTexture").map_or(1.0, |n| num(n, "strength", 1.0)) as f32,
            emissive: [(em[0] * strength) as f32, (em[1] * strength) as f32, (em[2] * strength) as f32],
            emissive_tex: tref(m.get("emissiveTexture")),
            alpha_mode: match m.get("alphaMode").and_then(Value::as_str) {
                Some("MASK") => AlphaMode::Mask,
                Some("BLEND") => AlphaMode::Blend,
                _ => AlphaMode::Opaque,
            },
            alpha_cutoff: num(m, "alphaCutoff", 0.5) as f32,
            double_sided: m.get("doubleSided").and_then(Value::as_bool).unwrap_or(false),
            unlit: ext.and_then(|e| e.get("KHR_materials_unlit")).is_some(),
        });
    }

    let mut meshes = vec![];
    for m in arr(&j, "meshes") {
        let mut mesh = Mesh { name: m.get("name").and_then(Value::as_str).unwrap_or("").to_string(), primitives: vec![] };
        for p in arr(m, "primitives") {
            if let Some(prim) = primitive(&d, p, &materials)? {
                mesh.primitives.push(prim);
            }
        }
        meshes.push(mesh);
    }

    let mut nodes = vec![];
    for n in arr(&j, "nodes") {
        let matrix = n.get("matrix").and_then(Value::as_array).filter(|a| a.len() == 16).map(|a| {
            // Column-major in the file.
            let v: Vec<f64> = a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
            let mut m = [[0.0; 4]; 4];
            for c in 0..4 {
                for r in 0..4 {
                    m[r][c] = v[c * 4 + r];
                }
            }
            Mat4(m)
        });
        nodes.push(Node {
            name: n.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            translation: nums(n, "translation", [0.0; 3]),
            rotation: nums(n, "rotation", [0.0, 0.0, 0.0, 1.0]),
            scale: nums(n, "scale", [1.0; 3]),
            matrix,
            mesh: idx(n, "mesh").filter(|&m| m < meshes.len()),
            skin: idx(n, "skin"),
            camera: idx(n, "camera"),
            light: n.get("extensions").and_then(|e| e.get("KHR_lights_punctual")).and_then(|e| idx(e, "light")),
            children: arr(n, "children").iter().filter_map(Value::as_u64).map(|c| c as usize).collect(),
        });
    }

    let scene = idx(&j, "scene").unwrap_or(0);
    let roots: Vec<usize> = match arr(&j, "scenes").get(scene) {
        Some(s) => arr(s, "nodes").iter().filter_map(Value::as_u64).map(|c| c as usize).collect(),
        None => {
            // No scene: every node that is nobody's child.
            let mut child = vec![false; nodes.len()];
            for n in &nodes {
                for &c in &n.children {
                    if c < child.len() {
                        child[c] = true;
                    }
                }
            }
            (0..nodes.len()).filter(|&i| !child[i]).collect()
        }
    };

    let mut skins = vec![];
    for s in arr(&j, "skins") {
        let joints: Vec<usize> = arr(s, "joints").iter().filter_map(Value::as_u64).map(|c| c as usize).collect();
        let inverse_bind = match idx(s, "inverseBindMatrices") {
            Some(a) => {
                let (f, _) = d.floats(a)?;
                let matrices = f.as_chunks::<16>().0;
                let mut out = d.geometry_vec(matrices.len())?;
                out.extend(matrices.iter().map(|v| {
                    let mut m = [[0.0; 4]; 4];
                    for c in 0..4 {
                        for r in 0..4 {
                            m[r][c] = v[c * 4 + r] as f64;
                        }
                    }
                    Mat4(m)
                }));
                out
            }
            None => {
                let mut out = d.geometry_vec(joints.len())?;
                out.resize(joints.len(), Mat4::IDENTITY);
                out
            }
        };
        skins.push(Skin { joints, inverse_bind });
    }

    let mut animations = vec![];
    for a in arr(&j, "animations") {
        let samplers = arr(a, "samplers");
        let mut anim = Animation { name: a.get("name").and_then(Value::as_str).unwrap_or("").to_string(), channels: vec![] };
        for c in arr(a, "channels") {
            let Some(t) = c.get("target") else { continue };
            let Some(node) = idx(t, "node") else { continue };
            let path = match t.get("path").and_then(Value::as_str) {
                Some("translation") => ChannelPath::Translation,
                Some("rotation") => ChannelPath::Rotation,
                Some("scale") => ChannelPath::Scale,
                _ => continue,
            };
            let Some(s) = idx(c, "sampler").and_then(|s| samplers.get(s)) else { continue };
            let interp = match s.get("interpolation").and_then(Value::as_str) {
                Some("STEP") => Interp::Step,
                Some("CUBICSPLINE") => Interp::CubicSpline,
                _ => Interp::Linear,
            };
            let (Some(input), Some(output)) = (idx(s, "input"), idx(s, "output")) else { continue };
            let (times, _) = d.floats(input)?;
            let (values, _) = d.floats(output)?;
            anim.channels.push(Channel { node, path, interp, times, values });
        }
        animations.push(anim);
    }

    let cameras = arr(&j, "cameras").iter().enumerate().map(|(i, c)| camera(c, i)).collect();
    let lights = j
        .get("extensions")
        .and_then(|e| e.get("KHR_lights_punctual"))
        .map(|e| arr(e, "lights").iter().enumerate().map(|(i, l)| light(l, i)).collect())
        .unwrap_or_default();
    Ok(Model { meshes, nodes, roots, materials, textures, skins, animations, cameras, lights })
}

fn name_or(v: &Value, d: String) -> String {
    v.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or(d)
}

/// A glTF camera (glTF 2.0 §5.12): `perspective` {yfov, aspectRatio?, znear} or
/// `orthographic` {xmag, ymag}.
fn camera(c: &Value, i: usize) -> crate::ModelCamera {
    let projection = match (c.get("type").and_then(Value::as_str), c.get("orthographic")) {
        (Some("orthographic"), Some(o)) => crate::CameraProjection::Orthographic { xmag: num(o, "xmag", 1.0), ymag: num(o, "ymag", 1.0) },
        _ => {
            let p = c.get("perspective").cloned().unwrap_or(Value::Null);
            crate::CameraProjection::Perspective {
                yfov: num(&p, "yfov", 0.8).clamp(1e-3, 3.1),
                aspect: p.get("aspectRatio").and_then(Value::as_f64),
                znear: num(&p, "znear", 0.01),
            }
        }
    };
    crate::ModelCamera { name: name_or(c, format!("Camera {}", i + 1)), projection }
}

/// A `KHR_lights_punctual` light: type, color, intensity, range, spot cone angles.
fn light(l: &Value, i: usize) -> crate::ModelLight {
    let kind = match l.get("type").and_then(Value::as_str) {
        Some("directional") => crate::ModelLightKind::Directional,
        Some("spot") => {
            let s = l.get("spot").cloned().unwrap_or(Value::Null);
            let outer = num(&s, "outerConeAngle", std::f64::consts::FRAC_PI_4).clamp(1e-3, std::f64::consts::FRAC_PI_2);
            crate::ModelLightKind::Spot { inner: num(&s, "innerConeAngle", 0.0).clamp(0.0, outer), outer }
        }
        _ => crate::ModelLightKind::Point,
    };
    crate::ModelLight {
        name: name_or(l, format!("Light {}", i + 1)),
        kind,
        color: nums(l, "color", [1.0; 3]),
        intensity: num(l, "intensity", 1.0).max(0.0),
        range: l.get("range").and_then(Value::as_f64),
    }
}

fn primitive(d: &Doc, p: &Value, materials: &[Material]) -> Result<Option<Primitive>> {
    let mode = idx(p, "mode").unwrap_or(4);
    if !matches!(mode, 4..=6) {
        return Ok(None);
    }
    let attrs = p.get("attributes").ok_or_else(|| perr("primitive without attributes"))?;
    let Some(pa) = idx(attrs, "POSITION") else { return Ok(None) };
    let positions = d.float_rows::<3>(Some(pa))?;
    let n = positions.len();
    let mut normals = d.float_rows::<3>(idx(attrs, "NORMAL"))?;
    let uvs = d.float_rows::<2>(idx(attrs, "TEXCOORD_0"))?;
    let tangents = d.float_rows::<4>(idx(attrs, "TANGENT"))?;
    let joints: Vec<[u16; 4]> = match idx(attrs, "JOINTS_0") {
        Some(a) => {
            let values = d.ints(a)?;
            let rows = values.as_chunks::<4>().0;
            let mut out = d.geometry_vec(rows.len())?;
            out.extend(rows.iter().map(|c| [c[0] as u16, c[1] as u16, c[2] as u16, c[3] as u16]));
            out
        }
        None => vec![],
    };
    let weights = d.float_rows::<4>(idx(attrs, "WEIGHTS_0"))?;
    let raw: Vec<u32> = match idx(p, "indices") {
        Some(a) => d.ints(a)?,
        None => {
            let mut out = d.geometry_vec(n)?;
            let end = u32::try_from(n).map_err(|_| perr("too many model vertices"))?;
            out.extend(0..end);
            out
        }
    };
    if raw.iter().any(|&i| i as usize >= n) {
        return Err(perr("index out of range"));
    }
    let index_count = match mode {
        5 | 6 => raw.len().saturating_sub(2).checked_mul(3).ok_or_else(|| perr("triangle count overflow"))?,
        _ => raw.len() / 3 * 3,
    };
    let mut indices = d.geometry_vec(index_count)?;
    match mode {
        5 => {
            for i in 2..raw.len() {
                let (a, b, c) = (raw[i - 2], raw[i - 1], raw[i]);
                if i % 2 == 0 { indices.extend([a, b, c]) } else { indices.extend([b, a, c]) }
            }
        }
        6 => {
            for i in 2..raw.len() {
                indices.extend([raw[0], raw[i - 1], raw[i]]);
            }
        }
        _ => indices.extend(raw.as_chunks::<3>().0.iter().flatten()),
    }
    if normals.len() != n {
        normals.clear();
    }
    let material = idx(p, "material").filter(|&m| m < materials.len());
    let skinned = joints.len() == n && weights.len() == n;
    let mut prim = Primitive {
        positions,
        normals,
        uvs: if uvs.len() == n { uvs } else { vec![] },
        tangents: if tangents.len() == n { tangents } else { vec![] },
        joints: if skinned { joints } else { vec![] },
        weights: if skinned { weights } else { vec![] },
        indices,
        material,
    };
    if prim.normals.is_empty() {
        let count = prim.indices.len();
        let flat = Primitive {
            positions: d.geometry_vec(count)?,
            normals: d.geometry_vec(count)?,
            indices: d.geometry_vec(count)?,
            uvs: if prim.uvs.is_empty() { vec![] } else { d.geometry_vec(count)? },
            joints: if prim.joints.is_empty() { vec![] } else { d.geometry_vec(count)? },
            weights: if prim.weights.is_empty() { vec![] } else { d.geometry_vec(count)? },
            ..Default::default()
        };
        prim.flat_normals_reserved(flat);
        prim.tangents.clear();
    }
    let needs_tangents = material.is_some_and(|m| materials[m].normal_tex.is_some());
    if needs_tangents && prim.tangents.is_empty() && prim.uvs.len() == prim.positions.len() {
        let count = prim.positions.len();
        let mut tan = d.geometry_vec(count)?;
        tan.resize(count, [0.0; 3]);
        let mut bit = d.geometry_vec(count)?;
        bit.resize(count, [0.0; 3]);
        prim.compute_tangents_reserved(tan, bit, d.geometry_vec(count)?);
    }
    Ok(Some(prim))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_buffers_share_the_model_budget_and_identical_samplers_share_pixels() {
        use image::ImageEncoder;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255], 2, 2, image::ExtendedColorType::Rgba8)
            .unwrap();
        let resolve = |uri: &str| (uri == "pixels.png").then(|| png.clone());
        let doc = serde_json::json!({
            "asset": {"version": "2.0"},
            "images": [{"uri": "pixels.png"}],
            "samplers": [{"wrapS": 33071, "wrapT": 10497}],
            "textures": [{"source": 0, "sampler": 0}, {"source": 0, "sampler": 0}]
        });
        let bytes = doc.to_string();
        // 16 native PNG bytes + 64 float conversion + 64 final RGBA + 64 Clamp copy.
        let model = parse_with_budget(bytes.as_bytes(), &resolve, png.len() + 208).unwrap();
        assert_eq!(model.textures.len(), 2);
        assert_eq!(model.textures[0].data.len(), 4);
        assert_eq!(model.textures[0].wrap, [Wrap::Clamp, Wrap::Repeat]);
        assert!(Arc::ptr_eq(&model.textures[0], &model.textures[1]));
        assert!(matches!(parse_with_budget(bytes.as_bytes(), &resolve, png.len() + 207), Err(ModelError::Unsupported(_))));
        assert!(matches!(parse_with_budget(bytes.as_bytes(), &resolve, png.len() + 143), Err(ModelError::Unsupported(_))));

        let mut doc = doc;
        doc["samplers"].as_array_mut().unwrap().push(serde_json::json!({"wrapS": 33648, "wrapT": 10497}));
        doc["textures"][1]["sampler"] = serde_json::json!(1);
        let model = parse_with_budget(doc.to_string().as_bytes(), &resolve, png.len() + 272).unwrap();
        assert!(!Arc::ptr_eq(&model.textures[0], &model.textures[1]));
        assert_eq!(model.textures[0].data, model.textures[1].data);
        assert_eq!(model.textures[1].wrap, [Wrap::Mirror, Wrap::Repeat]);
        assert!(matches!(parse_with_budget(doc.to_string().as_bytes(), &resolve, png.len() + 271), Err(ModelError::Unsupported(_))));
    }

    #[test]
    fn inverse_bind_matrix_storage_counts_toward_decode_budget() {
        let doc = serde_json::json!({
            "asset": {"version": "2.0"},
            "accessors": [{"componentType": 5126, "count": 1, "type": "MAT4"}],
            "nodes": [{}],
            "skins": [{"joints": [0], "inverseBindMatrices": 0}]
        });
        let bytes = doc.to_string();
        assert!(matches!(parse_with_budget(bytes.as_bytes(), &|_| None, 64), Err(ModelError::Unsupported(_))));
        assert!(matches!(parse_with_budget(bytes.as_bytes(), &|_| None, 191), Err(ModelError::Unsupported(_))));
        let model = parse_with_budget(bytes.as_bytes(), &|_| None, 192).unwrap();
        assert_eq!(model.skins[0].inverse_bind, vec![Mat4([[0.0; 4]; 4])]);
    }

    #[test]
    fn identity_bind_matrix_storage_counts_toward_decode_budget() {
        let doc = serde_json::json!({"asset": {"version": "2.0"}, "nodes": [{}], "skins": [{"joints": [0]}]});
        let bytes = doc.to_string();
        assert!(matches!(parse_with_budget(bytes.as_bytes(), &|_| None, 0), Err(ModelError::Unsupported(_))));
        assert!(matches!(parse_with_budget(bytes.as_bytes(), &|_| None, 127), Err(ModelError::Unsupported(_))));
        let model = parse_with_budget(bytes.as_bytes(), &|_| None, 128).unwrap();
        assert_eq!(model.skins[0].inverse_bind, vec![Mat4::IDENTITY]);
    }

    #[test]
    fn derived_buffer_limits_fail_before_reserving() {
        let doc = Doc { j: &Value::Null, buffers: vec![], decoded_remaining: Cell::new(64) };
        assert!(doc.geometry_vec::<u32>(17).is_err());
        assert_eq!(doc.decoded_remaining.get(), 64);
        assert!(doc.geometry_vec::<[f32; 3]>(MAX_ACCESSOR_BYTES / 12 + 1).is_err());
        assert!(doc.geometry_vec::<[f32; 4]>(usize::MAX).is_err());
        assert_eq!(doc.decoded_remaining.get(), 64);
    }

    fn square(mode: u32) -> (Value, Vec<u8>, Value) {
        let positions = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let order: [u32; 4] = if mode == 5 { [0, 1, 3, 2] } else { [0, 1, 2, 3] };
        let mut bytes = Vec::new();
        for position in positions {
            for component in position {
                bytes.extend(component.to_le_bytes());
            }
        }
        for index in order {
            bytes.extend(index.to_le_bytes());
        }
        for position in positions {
            for component in &position[..2] {
                bytes.extend(component.to_le_bytes());
            }
        }
        let json = serde_json::json!({
            "bufferViews": [
                {"buffer": 0, "byteLength": 48},
                {"buffer": 0, "byteOffset": 48, "byteLength": 16},
                {"buffer": 0, "byteOffset": 64, "byteLength": 32}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "type": "VEC3", "count": 4},
                {"bufferView": 1, "componentType": 5125, "type": "SCALAR", "count": 4},
                {"bufferView": 2, "componentType": 5126, "type": "VEC2", "count": 4}
            ]
        });
        let primitive = serde_json::json!({"attributes": {"POSITION": 0, "TEXCOORD_0": 2}, "indices": 1, "mode": mode, "material": 0});
        (json, bytes, primitive)
    }

    #[test]
    fn strip_fan_flat_normals_and_tangents_share_the_budget() {
        let materials = [Material { normal_tex: Some(TexRef { texture: 0, uv_set: 0 }), ..Default::default() }];
        for mode in [5, 6] {
            let (json, bytes, description) = square(mode);
            // Accessors total only 96 bytes. Derived flat vertices and tangent scratch must
            // also fit, even though their source buffers fit the budget.
            for budget in [400, 600] {
                let doc = Doc { j: &json, buffers: vec![bytes.clone()], decoded_remaining: Cell::new(budget) };
                assert!(matches!(primitive(&doc, &description, &materials), Err(ModelError::Unsupported(_))));
            }
            let doc = Doc { j: &json, buffers: vec![bytes], decoded_remaining: Cell::new(4096) };
            let p = primitive(&doc, &description, &materials).unwrap().unwrap();
            assert_eq!(p.triangle_count(), 2);
            assert_eq!(p.indices, [0, 1, 2, 3, 4, 5]);
            assert_eq!(p.positions.len(), 6);
            assert_eq!(p.uvs.len(), 6);
            assert_eq!(p.tangents.len(), 6);
            assert!(p.normals.iter().all(|normal| *normal == [0.0, 0.0, 1.0]));
            assert!(p.tangents.iter().all(|tangent| *tangent == [1.0, 0.0, 0.0, 1.0]));
        }
    }

    #[test]
    fn decoded_accessor_budget_is_shared() {
        let j = Value::Null;
        let doc = Doc { j: &j, buffers: vec![], decoded_remaining: Cell::new(16) };
        assert_eq!(doc.accessor_len(3, 1).unwrap(), 3);
        assert!(doc.accessor_len(2, 1).is_err());
        assert!(doc.accessor_len(usize::MAX, 16).is_err());
    }

    #[test]
    fn retained_external_buffers_share_the_import_budget() {
        let doc = br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"a.bin","byteLength":5},{"uri":"b.bin","byteLength":5}]}"#;
        let resolve = |name: &str| matches!(name, "a.bin" | "b.bin").then(|| vec![0; 5]);
        assert!(parse_with_budget(doc, &resolve, 10).is_ok(), "exact-size positive control");
        assert!(
            matches!(parse_with_budget(doc, &resolve, 9), Err(ModelError::Unsupported(_))),
            "two retained five-byte buffers must exceed a nine-byte shared import budget"
        );
    }

    #[test]
    fn copied_glb_bin_shares_the_import_budget() {
        let mut json = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":8}]}"#.to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut glb = Vec::new();
        glb.extend(GLB_MAGIC.to_le_bytes());
        glb.extend(2u32.to_le_bytes());
        glb.extend((12u32 + 8 + json.len() as u32 + 8 + 8).to_le_bytes());
        glb.extend((json.len() as u32).to_le_bytes());
        glb.extend(CHUNK_JSON.to_le_bytes());
        glb.extend(&json);
        glb.extend(8u32.to_le_bytes());
        glb.extend(CHUNK_BIN.to_le_bytes());
        glb.extend([0; 8]);
        assert!(parse_with_budget(&glb, &|_| None, 8).is_ok());
        assert!(matches!(parse_with_budget(&glb, &|_| None, 7), Err(ModelError::Unsupported(_))), "copied eight-byte BIN must exceed a seven-byte budget");
    }

    #[test]
    fn decoded_data_uri_shares_the_import_budget() {
        let uri = br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"data:application/octet-stream;base64,AAAA AAAA AAA=","byteLength":8}]}"#;
        assert!(parse_with_budget(uri, &|_| None, 8).is_ok());
        assert!(matches!(parse_with_budget(uri, &|_| None, 7), Err(ModelError::Unsupported(_))), "decoded eight-byte data URI must exceed a seven-byte budget");
    }

    #[test]
    fn encoded_image_copy_shares_the_import_budget() {
        use image::ImageEncoder;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8).unwrap();
        let doc = br#"{"asset":{"version":"2.0"},"images":[{"uri":"pixel.png"}],"textures":[{"source":0}]}"#;
        let resolve = |name: &str| (name == "pixel.png").then(|| png.clone());
        // Decoder RGBA8, float conversion and final float RGBA storage: 4 + 16 + 16.
        assert!(parse_with_budget(doc, &resolve, png.len() + 36).is_ok());
        assert!(
            matches!(parse_with_budget(doc, &resolve, 36), Err(ModelError::Unsupported(_))),
            "owned encoded PNG must consume budget in addition to its decoded pixels"
        );
    }

    #[test]
    fn whitespace_does_not_inflate_base64_output_capacity() {
        let text = " ".repeat(32) + "AA==";
        let decoded = base64_decode(&text).unwrap();
        assert_eq!(decoded, [0]);
        assert_eq!(decoded.capacity(), 1);
        assert!(base64_decode("AA!?").is_none());
    }

    #[test]
    fn retained_resolver_capacity_is_counted() {
        let doc = br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"a.bin","byteLength":1}]}"#;
        let resolve = |_: &str| {
            let mut bytes = Vec::with_capacity(32);
            bytes.push(0);
            Some(bytes)
        };
        assert!(parse_with_budget(doc, &resolve, 32).is_ok());
        assert!(matches!(parse_with_budget(doc, &resolve, 31), Err(ModelError::Unsupported(_))));
    }

    #[test]
    fn percent_data_uri_outputs_and_copy_capacity_are_preserved() {
        let doc = Doc { j: &Value::Null, buffers: vec![], decoded_remaining: Cell::new(14) };
        let ascii = doc.load_uri("data:text/plain,a%20b%2Bc", &|_| None).unwrap();
        assert_eq!(ascii, b"a b+c");
        assert_eq!(ascii.capacity(), 5);
        let doc = Doc { j: &Value::Null, buffers: vec![], decoded_remaining: Cell::new(14) };
        // Two invalid bytes followed by an incomplete four-byte UTF-8 sequence: three replacements.
        let lossy = doc.load_uri("data:text/plain,%FF%FE%F0%9F%92", &|_| None).unwrap();
        assert_eq!(lossy, "\u{FFFD}\u{FFFD}\u{FFFD}".as_bytes());
        assert_eq!(lossy.capacity(), 9);
        assert_eq!(doc.decoded_remaining.get(), 0); // Five scratch bytes + nine output bytes.
        let doc = Doc { j: &Value::Null, buffers: vec![], decoded_remaining: Cell::new(13) };
        assert!(matches!(doc.load_uri("data:text/plain,%FF%FE%F0%9F%92", &|_| None), Err(ModelError::Unsupported(_))));
    }

    #[test]
    fn buffer_view_image_copy_is_charged_separately_from_retained_source() {
        use image::ImageEncoder;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8).unwrap();
        let doc = serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"uri":"image.bin","byteLength":png.len()}],"bufferViews":[{"buffer":0,"byteLength":png.len()}],"images":[{"bufferView":0}],"textures":[{"source":0}]});
        let encoded = doc.to_string();
        let resolve = |name: &str| (name == "image.bin").then(|| png.clone());
        let budget = 2 * png.len() + 36;
        assert!(parse_with_budget(encoded.as_bytes(), &resolve, budget).is_ok());
        assert!(matches!(parse_with_budget(encoded.as_bytes(), &resolve, budget - 1), Err(ModelError::Unsupported(_))));
    }
}
