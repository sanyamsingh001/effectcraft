//! Distort effects, batch 2: Turbulent Displace, Displacement Map (self as map), Optics
//! Compensation, Magnify, Mesh Warp, Bezier Warp, CC Bend It, CC Lens, CC Page Turn, CC Tiler,
//! CC Griddler.
//!
//! Mesh Warp and Bezier Warp share a grid rasteriser: a (nx+1)×(ny+1) grid of destination points
//! and matching source points is split into triangles, bucketed per output row and filled
//! row-parallel with inverse barycentric sampling.
//!
//! Mesh Warp stores vertex offsets and four Bezier tangent adjustments in `mesh`. The viewer
//! edits the same bounded mesh the renderer samples. Legacy `dx,dy` strings remain readable.

use std::f64::consts::{FRAC_PI_2, PI};

use effectcraft_color::{BlendMode, blend_pixel};
use effectcraft_keyframe::{Keyframe, ShapePath, Value};
use effectcraft_project::{ParamUi, Property};
use effectcraft_raster::{Image, Px};
use effectcraft_time::Tick;
use rayon::prelude::*;

use crate::generate::value_noise;
use crate::util::{SRC_NAMES, Src, fit_layer, layer_rect, lerp4, map_xy, pick, pin_rect, premul, remap, smoothstep, src_at, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Distort", params, render, gpu: false, float: true }
}

fn pt(x: f64, y: f64) -> Value {
    Value::Vec2([x, y])
}

fn pct(max: f64) -> ParamUi {
    slider(0.0, max, 0.0, max.min(100.0), 1)
}

// ---------------------------------------------------------------- Turbulent Displace

fn fbm(u: f64, v: f64, z: f64, seed: u32, octaves: f64, falloff: f32) -> f32 {
    let n = octaves.ceil().max(1.0) as usize;
    let frac = (octaves - octaves.floor()) as f32;
    let (mut sum, mut norm, mut amp, mut f) = (0.0f32, 0.0f32, 1.0f32, 1.0f32);
    for o in 0..n {
        let w = if o + 1 == n && frac > 0.0 { frac } else { 1.0 };
        sum += value_noise(u as f32 * f, v as f32 * f, z as f32 + o as f32 * 5.17, seed.wrapping_add(o as u32 * 101)) * amp * w;
        norm += amp * w;
        amp *= falloff;
        f *= 2.0;
    }
    sum / norm.max(1e-6)
}

const TURBULENT_KINDS: [&str; 9] = [
    "Turbulent",
    "Bulge",
    "Twist",
    "Turbulent Smoother",
    "Bulge Smoother",
    "Twist Smoother",
    "Vertical Displacement",
    "Horizontal Displacement",
    "Cross Displacement",
];
/// Turbulent Displace's Pinning options. The Locked variants also keep every displaced
/// sample inside the layer (no edge pixels pulled in from outside) and ease the pinning over a
/// wider band, so the pinned edges and their neighbourhood stay put.
const TURBULENT_PINNING: [&str; 15] = [
    "None",
    "Pin All",
    "Pin Horizontal",
    "Pin Vertical",
    "Pin Left",
    "Pin Top",
    "Pin Right",
    "Pin Bottom",
    "Pin All Locked",
    "Pin Horizontal Locked",
    "Pin Vertical Locked",
    "Pin Left Locked",
    "Pin Top Locked",
    "Pin Right Locked",
    "Pin Bottom Locked",
];

fn turbulent_displace(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let amount = ctx.params.f("amount") * b.scale;
    if amount.abs() < 1e-9 {
        return b;
    }
    // TURBULENT_KINDS order: the "Smoother" variants (3, 4, 5) use a gentler octave falloff.
    let kind = ctx.params.e("displacement");
    let size = (ctx.params.f("size") * b.scale).max(1.0);
    if ctx.params.b("resizeLayer") && !ctx.adjustment {
        b.pad(amount.abs().ceil() as u32 + 1);
    }
    let off = b.to_px(ctx.params.v2("offset"));
    let oct = ctx.params.f("complexity").clamp(1.0, 10.0);
    let evo = ctx.params.f("evolution") / 360.0;
    // Evolution Options: Cycle Evolution loops the noise every `cycle` revolutions.
    let cycle = (ctx.params.b("evolutionOptions/cycleEvolution")).then(|| ctx.params.f("evolutionOptions/cycle").round().max(1.0));
    let pin_raw = ctx.params.e("pinning");
    let locked = pin_raw >= 8;
    let pin = if locked { pin_raw - 7 } else { pin_raw };
    let falloff = if (3..=5).contains(&kind) { 0.3 } else { 0.5 };
    let (lx, ly, lw, lh) = pin_rect(ctx, b.offset, b.scale);
    let edge = (lw.min(lh) * if locked { 0.25 } else { 0.1 }).max(1.0);
    let seed = (ctx.seed ^ 0x7d15).wrapping_add((ctx.params.f("evolutionOptions/randomSeed") as i64 as u32).wrapping_mul(0x9e37_79b9));
    let n = |u: f64, v: f64, s: u32| {
        let s = seed.wrapping_add(s);
        match cycle {
            Some(c) => {
                // Cross-fade the noise at z and z - c so z = 0 and z = c match.
                let z = evo.rem_euclid(c);
                let t = z / c;
                (fbm(u, v, z, s, oct, falloff) as f64 * (1.0 - t) + fbm(u, v, z - c, s, oct, falloff) as f64 * t) - 0.5
            }
            None => fbm(u, v, evo, s, oct, falloff) as f64 - 0.5,
        }
    };
    b.img = remap(&b.img, false, |x, y| {
        let (u, v) = ((x - off.0) / size, (y - off.1) / size);
        let (dx, dy) = match kind {
            1 | 2 | 4 | 5 => {
                let e = 0.05;
                let gx = (n(u + e, v, 0) - n(u - e, v, 0)) / (2.0 * e);
                let gy = (n(u, v + e, 0) - n(u, v - e, 0)) / (2.0 * e);
                let k = amount * 0.25;
                if kind == 1 || kind == 4 { (gx * k, gy * k) } else { (-gy * k, gx * k) }
            }
            6 => (0.0, n(u, v, 17) * amount),
            7 => (n(u, v, 0) * amount, 0.0),
            8 => {
                let a = n(u, v, 0) * amount;
                (a, a)
            }
            _ => (n(u, v, 0) * amount, n(u, v, 17) * amount),
        };
        // Measured from the outermost pixel centres, so pinned edges stay exactly in place.
        let (l, r) = (((x - lx - 0.5) / edge).clamp(0.0, 1.0), ((lx + lw - 0.5 - x) / edge).clamp(0.0, 1.0));
        let (t, bt) = (((y - ly - 0.5) / edge).clamp(0.0, 1.0), ((ly + lh - 0.5 - y) / edge).clamp(0.0, 1.0));
        let k = match pin {
            1 => l.min(r).min(t).min(bt),
            2 => t.min(bt),
            3 => l.min(r),
            4 => l,
            5 => t,
            6 => r,
            7 => bt,
            _ => 1.0,
        };
        let (sx, sy) = (x + dx * k, y + dy * k);
        if locked {
            return Some((sx.clamp(lx + 0.5, lx + lw - 0.5), sy.clamp(ly + 0.5, ly + lh - 0.5)));
        }
        Some((sx, sy))
    });
    b
}

// ---------------------------------------------------------------- Displacement Map

fn displacement_map(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let hs = src_at(ctx.params.e("useForHorizontal"));
    let vs = src_at(ctx.params.e("useForVertical"));
    let mh = ctx.params.f("maxHorizontal") * b.scale;
    let mv = ctx.params.f("maxVertical") * b.scale;
    let wrap = ctx.params.b("wrapPixelsAround");
    if ctx.params.b("expandOutput") && !wrap && !ctx.adjustment {
        b.pad(mh.abs().max(mv.abs()).ceil() as u32);
    }
    // The map: the chosen layer (Center / Stretch / Tile), else the layer itself.
    let layer = ctx.layer_param("displacementMapLayer", false);
    let behavior = ctx.params.e("displacementMapBehavior");
    let map_size = layer.as_ref().map(|o| o.size);
    let map = match layer {
        Some(o) => match behavior {
            1 => fit_layer(ctx, &b, &o, true),
            2 => {
                let (os, ls) = (o.size, ctx.layer_size);
                let (dx, dy) = ((os[0] - ls[0]) * 0.5, (os[1] - ls[1]) * 0.5);
                let inv = 1.0 / b.scale.max(1e-9);
                crate::util::gen_image(b.img.width, b.img.height, |x, y| {
                    let qx = ((x as f64 + 0.5 - b.offset[0]) * inv + dx).rem_euclid(os[0].max(1.0));
                    let qy = ((y as f64 + 0.5 - b.offset[1]) * inv + dy).rem_euclid(os[1].max(1.0));
                    o.buf.img.sample_bilinear_clamped(qx * o.buf.scale + o.buf.offset[0], qy * o.buf.scale + o.buf.offset[1])
                })
            }
            _ => fit_layer(ctx, &b, &o, false),
        },
        None => b.img.clone(),
    };
    // Center Map: pixels outside the (smaller) map are not displaced.
    let (scale, offset) = (b.scale, b.offset);
    let outside = |x: usize, y: usize| -> bool {
        match map_size {
            Some(os) if behavior == 0 => {
                let inv = 1.0 / scale.max(1e-9);
                let ls = ctx.layer_size;
                let qx = (x as f64 + 0.5 - offset[0]) * inv + (os[0] - ls[0]) * 0.5;
                let qy = (y as f64 + 0.5 - offset[1]) * inv + (os[1] - ls[1]) * 0.5;
                !(0.0..os[0]).contains(&qx) || !(0.0..os[1]).contains(&qy)
            }
            _ => false,
        }
    };
    let src = b.img.clone();
    let (w, h) = (src.width as f64, src.height as f64);
    map_xy(&mut b.img, |x, y, px| {
        if outside(x, y) {
            return px;
        }
        let (c, a) = unpremul(map.get(x as i64, y as i64));
        // "Off" turns displacement in that direction off.
        let d = |s: Src, m: f64| if s == Src::Off { 0.0 } else { (pick(s, c, a) as f64 - 0.5) * 2.0 * m };
        let (dx, dy) = (d(hs, mh), d(vs, mv));
        if dx == 0.0 && dy == 0.0 {
            return px;
        }
        let (mut sx, mut sy) = (x as f64 + 0.5 + dx, y as f64 + 0.5 + dy);
        if wrap {
            sx = sx.rem_euclid(w);
            sy = sy.rem_euclid(h);
            src.sample_bilinear_clamped(sx, sy)
        } else {
            src.sample_bilinear(sx, sy)
        }
    });
    b
}

// ---------------------------------------------------------------- Optics Compensation

fn optics_compensation(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let fov = ctx.params.f("fieldOfView").clamp(0.0, 179.0).to_radians();
    if fov < 1e-6 {
        return b;
    }
    let reverse = ctx.params.b("reverseLensDistortion");
    let (lx, ly, lw, lh) = layer_rect(ctx, &b);
    let r_ref = match ctx.params.e("fovOrientation") {
        1 => lh * 0.5,
        2 => (lw * lw + lh * lh).sqrt() * 0.5,
        _ => lw * 0.5,
    }
    .max(1.0);
    let half = fov * 0.5;
    let f = r_ref / half.tan();
    // Resize (with Reverse Lens Distortion): grow the layer so the stretched corners stay
    // visible, up to 2× / 4× the layer size (Unlimited is capped at 8×).
    // Optimal Pixels (invalidates Resize): scale the result so the whole corrected picture
    // fits the layer (Reverse) or fills it with no empty corners (forward).
    let optimal = ctx.params.b("optimalPixels");
    let resize = if optimal { 0 } else { ctx.params.e("resize") };
    if reverse && resize > 0 && !ctx.adjustment {
        let c = b.to_px(ctx.params.v2("viewCenter"));
        let rc =
            [(lx, ly), (lx + lw, ly), (lx, ly + lh), (lx + lw, ly + lh)].iter().map(|p| ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt()).fold(0.0, f64::max);
        let th = rc / r_ref * half;
        let cap = lw.max(lh) * [0.0, 0.5, 1.5, 3.5][resize.min(3) as usize];
        let grow = if th < FRAC_PI_2 - 1e-3 { (f * th.tan() - rc).max(0.0) } else { cap };
        b.pad(grow.min(cap).min(4096.0).ceil() as u32);
    }
    let c = b.to_px(ctx.params.v2("viewCenter"));
    let fit = if optimal {
        let rc =
            [(lx, ly), (lx + lw, ly), (lx, ly + lh), (lx + lw, ly + lh)].iter().map(|p| ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt()).fold(1.0, f64::max);
        if reverse {
            let th = (rc / r_ref * half).min(FRAC_PI_2 - 1e-3);
            (f * th.tan() / rc).max(1.0)
        } else {
            ((rc / f).atan() / half * r_ref / rc).min(1.0)
        }
    } else {
        1.0
    };
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let r = (dx * dx + dy * dy).sqrt() * fit;
        if r < 1e-9 {
            return Some((x, y));
        }
        let rs = if reverse {
            (r / f).atan() / half * r_ref
        } else {
            let th = r / r_ref * half;
            if th >= FRAC_PI_2 - 1e-4 {
                return None;
            }
            f * th.tan()
        };
        let k = rs / r * fit;
        Some((c.0 + dx * k, c.1 + dy * k))
    });
    b
}

// ---------------------------------------------------------------- Magnify

fn magnify(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let square = ctx.params.e("shape") == 1;
    let c = b.to_px(ctx.params.v2("center"));
    let mag = (ctx.params.f("magnification") / 100.0).max(0.01);
    let link = ctx.params.e("link");
    let mut size = ctx.params.f("size") * b.scale;
    let mut feather = ctx.params.f("feather") * b.scale;
    if link >= 1 {
        size *= mag;
    }
    if link == 2 {
        feather *= mag;
    }
    let op = (ctx.params.f("opacity") / 100.0) as f32;
    let scaling = ctx.params.e("scaling");
    // Blending Mode: index 0 is "None" (only the magnified area, transparent around it).
    let mode = ctx.params.e("blendingMode") as usize;
    let blend = MAGNIFY_MODES.get(mode.wrapping_sub(1)).copied();
    if ctx.params.b("resizeLayer") && link == 0 && !ctx.adjustment {
        // Let the magnified area extend past the layer edges.
        let (lx, ly, lw, lh) = layer_rect(ctx, &b);
        let reach = size.max(0.0);
        let over = [lx - (c.0 - reach), (c.0 + reach) - (lx + lw), ly - (c.1 - reach), (c.1 + reach) - (ly + lh)].into_iter().fold(0.0, f64::max);
        b.pad(over.min(4096.0).ceil() as u32);
    }
    let c = b.to_px(ctx.params.v2("center"));
    let src = b.img.clone();
    let seed = ctx.seed ^ 0x3a61;
    map_xy(&mut b.img, |x, y, px| {
        let (dx, dy) = (x as f64 + 0.5 - c.0, y as f64 + 0.5 - c.1);
        let d = if square { dx.abs().max(dy.abs()) } else { (dx * dx + dy * dy).sqrt() };
        let w = if feather > 0.01 { 1.0 - smoothstep((size - feather) as f32, size as f32, d as f32) } else { (size - d + 0.5).clamp(0.0, 1.0) as f32 } * op;
        if w <= 0.0 {
            return if blend.is_none() { [0.0; 4] } else { px };
        }
        let (sx, sy) = (c.0 + dx / mag, c.1 + dy / mag);
        let m = match scaling {
            // Soft: smooth (bicubic spline) interpolation.
            1 => src.sample_bicubic(sx, sy),
            // Scatter: jitter each sample within the enlarged source pixel.
            2 => {
                let j = |k: u32| (crate::util::hash1(x as u32, y as u32 ^ (k << 20), seed) as f64 - 0.5) * (1.0 - 1.0 / mag).max(0.0);
                src.sample_bilinear(sx.floor() + 0.5 + j(1), sy.floor() + 0.5 + j(2))
            }
            // Standard: nearest source pixel (sharp, blocky at high magnification).
            _ => src.sample_bilinear(sx.floor() + 0.5, sy.floor() + 0.5),
        };
        match blend {
            None => m.map(|v| v * w),
            Some(BlendMode::Normal) => lerp4(px, m, w),
            Some(mode) => blend_pixel(mode, px, m.map(|v| v * w), 0.5),
        }
    });
    b
}

/// Magnify's Blending Mode choices after "None".
pub const MAGNIFY_MODES: [BlendMode; 17] = [
    BlendMode::Normal,
    BlendMode::Add,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
];
const MAGNIFY_MODE_NAMES: [&str; 18] = [
    "None",
    "Normal",
    "Add",
    "Multiply",
    "Screen",
    "Overlay",
    "Soft Light",
    "Hard Light",
    "Color Dodge",
    "Color Burn",
    "Darken",
    "Lighten",
    "Difference",
    "Exclusion",
    "Hue",
    "Saturation",
    "Color",
    "Luminosity",
];

// ---------------------------------------------------------------- grid rasteriser

type Pt = (f64, f64);

// At most 16 MiB of indices for the supported grids, independent of output height or
// distorted triangle size. Small grids use up to 64 bins to keep row traversal cheap.
const GRID_BIN_INDEX_BUDGET: usize = 2 * 1024 * 1024;

struct GridBins {
    rows_per_bin: usize,
    bins: Vec<Vec<usize>>,
    triangle_rows: Vec<std::ops::Range<usize>>,
}

fn grid_bins(triangles: &[[usize; 3]], dest: &[Pt], height: usize) -> GridBins {
    let max_bins = (GRID_BIN_INDEX_BUDGET / triangles.len().max(1)).clamp(1, 64);
    let rows_per_bin = height.div_ceil(max_bins).max(1);
    let bin_count = height.div_ceil(rows_per_bin);
    let triangle_rows: Vec<_> = triangles
        .iter()
        .map(|tri| {
            let ys = tri.map(|k| dest[k].1);
            let first = ((ys[0].min(ys[1]).min(ys[2]) - 0.5).floor().max(0.0) as usize).min(height);
            let end = ((ys[0].max(ys[1]).max(ys[2]) + 0.5).ceil().max(0.0) as usize).min(height);
            first..end
        })
        .collect();
    let bins_for = |rows: &std::ops::Range<usize>| {
        let first = rows.start / rows_per_bin;
        let end = if rows.is_empty() { first } else { (rows.end - 1) / rows_per_bin + 1 };
        first..end
    };
    // Count first so every bin has exact capacity rather than growing to twice its size.
    let mut counts = vec![0; bin_count];
    for rows in &triangle_rows {
        for bin in bins_for(rows) {
            counts[bin] += 1;
        }
    }
    let mut bins: Vec<Vec<usize>> = counts.into_iter().map(Vec::with_capacity).collect();
    for (t, rows) in triangle_rows.iter().enumerate() {
        for bin in bins_for(rows) {
            bins[bin].push(t);
        }
    }
    GridBins { rows_per_bin, bins, triangle_rows }
}

/// Warp `src` into a `w`×`h` image: grid point `i` (row-major, (nx+1)×(ny+1)) at `dest[i]` shows
/// the source at `srcp[i]`. Uncovered pixels are transparent.
fn grid_warp(src: &Image, w: u32, h: u32, nx: usize, ny: usize, dest: &[Pt], srcp: &[Pt]) -> Image {
    let idx = |i: usize, j: usize| j * (nx + 1) + i;
    let mut tris: Vec<[usize; 3]> = Vec::with_capacity(nx * ny * 2);
    for j in 0..ny {
        for i in 0..nx {
            let (a, b2, c, d) = (idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1));
            tris.push([a, b2, c]);
            tris.push([a, c, d]);
        }
    }
    let bins = grid_bins(&tris, dest, h as usize);
    let mut out = Image::new(w, h);
    out.rows_mut().for_each(|(y, row)| {
        let py = y as f64 + 0.5;
        for &t in &bins.bins[y / bins.rows_per_bin] {
            if !bins.triangle_rows[t].contains(&y) {
                continue;
            }
            let [ia, ib, ic] = tris[t];
            let (a, bb, c) = (dest[ia], dest[ib], dest[ic]);
            let den = (bb.1 - c.1) * (a.0 - c.0) + (c.0 - bb.0) * (a.1 - c.1);
            if !den.is_finite() || den.abs() < 1e-12 {
                continue;
            }
            let x0 = (a.0.min(bb.0).min(c.0) - 0.5).floor().max(0.0) as usize;
            let x1 = ((a.0.max(bb.0).max(c.0) + 0.5).ceil().max(0.0) as usize).min(row.len());
            for x in x0..x1 {
                if row[x][3] > 0.0 {
                    continue;
                }
                let px = x as f64 + 0.5;
                let l1 = ((bb.1 - c.1) * (px - c.0) + (c.0 - bb.0) * (py - c.1)) / den;
                let l2 = ((c.1 - a.1) * (px - c.0) + (a.0 - c.0) * (py - c.1)) / den;
                let l3 = 1.0 - l1 - l2;
                let e = -1e-9;
                if !(l1 >= e && l2 >= e && l3 >= e) {
                    continue;
                }
                let (sa, sb, sc) = (srcp[ia], srcp[ib], srcp[ic]);
                let sx = sa.0 * l1 + sb.0 * l2 + sc.0 * l3;
                let sy = sa.1 * l1 + sb.1 * l2 + sc.1 * l3;
                row[x] = src.sample_bilinear(sx, sy);
            }
        }
    });
    out
}

/// Mesh Warp's `mesh` parameter: the vertex offsets (layer pixels), row-major.
pub fn parse_mesh(s: &str) -> Vec<(f64, f64)> {
    s.split(|c: char| c.is_whitespace() || c == ';')
        .filter(|t| !t.is_empty())
        .take(1025)
        .filter_map(|t| {
            let mut it = t.split(',');
            let x = it.next()?.trim().parse::<f64>().ok()?;
            let y = it.next()?.trim().parse::<f64>().ok()?;
            (x.is_finite() && y.is_finite()).then_some((x, y))
        })
        .collect()
}

/// A distortion mesh in layer pixels. Each vertex carries its offset followed by adjustments
/// to the straight left, right, top and bottom tangents. Counts are bounded to 32 × 32 vertices.
#[derive(Clone, Debug, PartialEq)]
pub struct WarpMesh {
    pub rows: usize,
    pub columns: usize,
    pub controls: Vec<[[f64; 2]; 5]>,
}

impl WarpMesh {
    pub fn dimension(n: f64) -> usize {
        if n.is_finite() { n.round().clamp(1.0, 31.0) as usize } else { 7 }
    }

    pub fn identity(rows: usize, columns: usize) -> Self {
        let (rows, columns) = (rows.clamp(1, 31), columns.clamp(1, 31));
        Self { rows, columns, controls: vec![[[0.0; 2]; 5]; (rows + 1) * (columns + 1)] }
    }

    /// Read versioned data, or an old row-major offset list. Invalid data repairs to identity;
    /// parsing never allocates more than the maximum vertex count.
    pub fn parse(s: &str, rows: usize, columns: usize) -> Self {
        let mut out = Self::identity(rows, columns);
        let Some(rest) = s.strip_prefix("mw1;") else {
            let offsets = parse_mesh(s);
            if offsets.len() == out.controls.len() {
                for (v, (x, y)) in out.controls.iter_mut().zip(offsets) {
                    v[0] = [x, y];
                }
            }
            return out;
        };
        let Some((header, data)) = rest.split_once(';') else { return out };
        let Some((r, c)) = header.split_once(',') else { return out };
        let (Ok(r), Ok(c)) = (r.parse::<usize>(), c.parse::<usize>()) else { return out };
        if !(1..=31).contains(&r) || !(1..=31).contains(&c) {
            return out;
        }
        let mut original = Self::identity(r, c);
        let mut tokens = data.split_whitespace();
        for vertex in &mut original.controls {
            let Some(token) = tokens.next() else { return out };
            let mut numbers = token.split(',');
            for control in vertex {
                for value in control {
                    let Some(number) = numbers.next().and_then(|n| n.parse::<f64>().ok()).filter(|n| n.is_finite()) else { return out };
                    *value = number;
                }
            }
            if numbers.next().is_some() {
                return out;
            }
        }
        if tokens.next().is_some() {
            return out;
        }
        if (r, c) == (out.rows, out.columns) {
            return original;
        }
        // Changing Rows / Columns preserves the offset field instead of silently discarding it.
        for j in 0..=out.rows {
            for i in 0..=out.columns {
                let x = i as f64 / out.columns as f64 * c as f64;
                let y = j as f64 / out.rows as f64 * r as f64;
                let (ci, cj) = ((x.floor() as usize).min(c - 1), (y.floor() as usize).min(r - 1));
                let (u, v) = (x - ci as f64, y - cj as f64);
                if let Some(dst) = out.controls.get_mut(j * (out.columns + 1) + i) {
                    for (di, dj, weight) in [(0, 0, (1.0 - u) * (1.0 - v)), (1, 0, u * (1.0 - v)), (0, 1, (1.0 - u) * v), (1, 1, u * v)] {
                        if let Some(src) = original.controls.get((cj + dj) * (c + 1) + ci + di) {
                            for (to, from) in dst.iter_mut().zip(src) {
                                for (a, b) in to.iter_mut().zip(from) {
                                    *a += b * weight;
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    pub fn format(&self) -> String {
        let mut text = format!("mw1;{},{};", self.rows, self.columns);
        for vertex in &self.controls {
            if !text.ends_with(';') {
                text.push(' ');
            }
            text.push_str(&vertex.iter().flat_map(|p| p.iter()).map(f64::to_string).collect::<Vec<_>>().join(","));
        }
        text
    }

    pub fn is_identity(&self) -> bool {
        self.controls.iter().flatten().flatten().all(|n| *n == 0.0)
    }

    /// 0 is the vertex; 1–4 are left, right, top and bottom. Boundary vertices have only
    /// the tangent handles which lead to another vertex.
    pub fn point(&self, index: usize, part: usize, size: [f64; 2]) -> Option<[f64; 2]> {
        if !(1..=31).contains(&self.rows) || !(1..=31).contains(&self.columns) {
            return None;
        }
        let vertex = self.controls.get(index)?;
        let (i, j) = (index % (self.columns + 1), index / (self.columns + 1));
        let position = [i as f64 / self.columns as f64 * size[0] + vertex[0][0], j as f64 / self.rows as f64 * size[1] + vertex[0][1]];
        if part == 0 {
            return Some(position);
        }
        let neighbor = match part {
            1 if i > 0 => index.checked_sub(1)?,
            2 if i < self.columns => index.checked_add(1)?,
            3 if j > 0 => index.checked_sub(self.columns + 1)?,
            4 if j < self.rows => index.checked_add(self.columns + 1)?,
            _ => return None,
        };
        let next = self.point(neighbor, 0, size)?;
        let adjustment = vertex.get(part)?;
        Some([position[0] + (next[0] - position[0]) / 3.0 + adjustment[0], position[1] + (next[1] - position[1]) / 3.0 + adjustment[1]])
    }

    /// Coons patch through the four Bezier edges of a cell, at normalized layer u/v.
    pub fn sample(&self, u: f64, v: f64, size: [f64; 2]) -> [f64; 2] {
        if !(1..=31).contains(&self.rows) || !(1..=31).contains(&self.columns) {
            return [0.0; 2];
        }
        let (x, y) = (u.clamp(0.0, 1.0) * self.columns as f64, v.clamp(0.0, 1.0) * self.rows as f64);
        let (i, j) = ((x.floor() as usize).min(self.columns - 1), (y.floor() as usize).min(self.rows - 1));
        let (u, v) = (x - i as f64, y - j as f64);
        let index = j * (self.columns + 1) + i;
        let (tl, tr, bl, br) = (index, index + 1, index + self.columns + 1, index + self.columns + 2);
        let at = |n, p| self.point(n, p, size).unwrap_or([0.0; 2]);
        let c0 = bez(at(tl, 0), at(tl, 2), at(tr, 1), at(tr, 0), u);
        let c1 = bez(at(bl, 0), at(bl, 2), at(br, 1), at(br, 0), u);
        let d0 = bez(at(tl, 0), at(tl, 4), at(bl, 3), at(bl, 0), v);
        let d1 = bez(at(tr, 0), at(tr, 4), at(br, 3), at(br, 0), v);
        let (tl, tr, bl, br) = (at(tl, 0), at(tr, 0), at(bl, 0), at(br, 0));
        [0, 1].map(|k| {
            let bil = tl[k] * (1.0 - u) * (1.0 - v) + tr[k] * u * (1.0 - v) + bl[k] * (1.0 - u) * v + br[k] * u * v;
            c0[k] * (1.0 - v) + c1[k] * v + d0[k] * (1.0 - u) + d1[k] * u - bil
        })
    }
}

/// Evaluate only the segment and its neighbors, using the existing path interpolation and
/// temporal easing rules for the mesh's numeric control pairs. Storage stays legacy-compatible.
pub fn mesh_value(prop: &Property, rows: usize, columns: usize, time: Tick) -> Value {
    let keys = &prop.keys;
    if keys.len() < 2 || keys.first().is_none_or(|k| time <= k.time) || keys.last().is_none_or(|k| time >= k.time) {
        return prop.value_at(time);
    }
    let i = keys.partition_point(|k| k.time <= time).saturating_sub(1);
    let start = i.saturating_sub(1);
    let end = i.saturating_add(3).min(keys.len());
    let Some(segment) = keys.get(start..end) else { return prop.value_at(time) };
    let transformed: Vec<Keyframe> = segment
        .iter()
        .map(|key| {
            let text = match &key.value {
                Value::Str(s) => s.as_str(),
                _ => "",
            };
            let mesh = WarpMesh::parse(text, rows, columns);
            let pairs: Vec<[f64; 2]> = mesh.controls.iter().flat_map(|p| p.iter().copied()).collect();
            let mut key = key.clone();
            key.value = Value::Path(ShapePath::polygon(&pairs, false));
            key
        })
        .collect();
    let Some(Value::Path(path)) = effectcraft_keyframe::evaluate(&transformed, time, false) else { return prop.value_at(time) };
    let mut mesh = WarpMesh::identity(rows, columns);
    for (vertex, pairs) in mesh.controls.iter_mut().zip(path.vertices.as_chunks::<5>().0) {
        for (to, from) in vertex.iter_mut().zip(pairs) {
            *to = *from;
        }
    }
    Value::Str(mesh.format())
}

fn mesh_warp(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let rows = WarpMesh::dimension(ctx.params.f("rows"));
    let cols = WarpMesh::dimension(ctx.params.f("columns"));
    // Three subdivisions resolve cubic edges even when opposing tangents cancel at their midpoint.
    let q = WarpMesh::dimension(ctx.params.f("quality")).clamp(3, 10);
    let mesh = WarpMesh::parse(ctx.params.s("mesh"), rows, cols);
    if mesh.is_identity() {
        return b;
    }
    let (lw, lh) = (ctx.layer_size[0], ctx.layer_size[1]);
    let (nx, ny) = (cols * q, rows * q);
    let mut dest = Vec::with_capacity((nx + 1) * (ny + 1));
    let mut srcp = Vec::with_capacity((nx + 1) * (ny + 1));
    for gj in 0..=ny {
        for gi in 0..=nx {
            let dest_point = mesh.sample(gi as f64 / nx as f64, gj as f64 / ny as f64, [lw, lh]);
            let origin = ctx.env.bounds_origin;
            dest.push(b.to_px([dest_point[0] + origin[0], dest_point[1] + origin[1]]));
            srcp.push(b.to_px([gi as f64 / nx as f64 * lw + origin[0], gj as f64 / ny as f64 * lh + origin[1]]));
        }
    }
    b.img = grid_warp(&b.img, b.img.width, b.img.height, nx, ny, &dest, &srcp);
    b
}

// ---------------------------------------------------------------- Bezier Warp

fn bez(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let (a, bb, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [p0[0] * a + p1[0] * bb + p2[0] * c + p3[0] * d, p0[1] * a + p1[1] * bb + p2[1] * c + p3[1] * d]
}

fn bezier_warp(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let (n, dest, srcp) = bezier_grid(ctx, b.offset, b.scale);
    b.img = grid_warp(&b.img, b.img.width, b.img.height, n, n, &dest, &srcp);
    b
}

/// Bezier Warp's n×n grid: (n, destination points, source points) in buffer pixels (the GPU
/// rasterises the same grid).
pub fn bezier_grid(ctx: &EffectCtx, offset: [f64; 2], scale: f64) -> (usize, Vec<Pt>, Vec<Pt>) {
    let to_px = |p: [f64; 2]| (p[0] * scale + offset[0], p[1] * scale + offset[1]);
    let g = |id: &str| ctx.params.v2(id);
    let (tl, tlt, trt, tr) = (g("topLeftVertex"), g("topLeftTangent"), g("topRightTangent"), g("rightTopVertex"));
    let (rtt, rbt, br) = (g("rightTopTangent"), g("rightBottomTangent"), g("bottomRightVertex"));
    let (brt, blt, bl) = (g("bottomRightTangent"), g("bottomLeftTangent"), g("leftBottomVertex"));
    let (lbt, ltt) = (g("leftBottomTangent"), g("leftTopTangent"));
    let n = (ctx.params.f("quality").round().clamp(1.0, 10.0) as usize) * 4;
    let (lw, lh) = (ctx.layer_size[0], ctx.layer_size[1]);
    let mut dest = Vec::with_capacity((n + 1) * (n + 1));
    let mut srcp = Vec::with_capacity((n + 1) * (n + 1));
    for j in 0..=n {
        let v = j as f64 / n as f64;
        let d0 = bez(tl, ltt, lbt, bl, v);
        let d1 = bez(tr, rtt, rbt, br, v);
        for i in 0..=n {
            let u = i as f64 / n as f64;
            let c0 = bez(tl, tlt, trt, tr, u);
            let c1 = bez(bl, blt, brt, br, u);
            let mut s = [0.0; 2];
            for k in 0..2 {
                let bil = (1.0 - u) * (1.0 - v) * tl[k] + u * (1.0 - v) * tr[k] + (1.0 - u) * v * bl[k] + u * v * br[k];
                s[k] = (1.0 - v) * c0[k] + v * c1[k] + (1.0 - u) * d0[k] + u * d1[k] - bil;
            }
            dest.push(to_px(s));
            srcp.push(to_px([u * lw, v * lh]));
        }
    }
    (n, dest, srcp)
}

// ---------------------------------------------------------------- CC Bend It

fn bend_it(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let mut beta = ctx.params.f("bend").to_radians();
    if beta.abs() < 1e-6 {
        return b;
    }
    let s = b.to_px(ctx.params.v2("start"));
    let e = b.to_px(ctx.params.v2("end"));
    let static_pre = ctx.params.e("renderPrestart") == 1;
    let l = ((e.0 - s.0).powi(2) + (e.1 - s.1).powi(2)).sqrt();
    if l < 1e-6 {
        return b;
    }
    let a = ((e.0 - s.0) / l, (e.1 - s.1) / l);
    let mut n = (-a.1, a.0);
    if beta < 0.0 {
        beta = -beta;
        n = (-n.0, -n.1);
    }
    let rho = l / beta;
    let pe = (rho * beta.sin(), rho - rho * beta.cos());
    let (t, nn) = ((beta.cos(), beta.sin()), (-beta.sin(), beta.cos()));
    let src = b.img.clone();
    let to_src = |u: f64, w: f64| (s.0 + a.0 * u + n.0 * w, s.1 + a.1 * u + n.1 * w);
    map_xy(&mut b.img, |x, y, _| {
        let (vx, vy) = (x as f64 + 0.5 - s.0, y as f64 + 0.5 - s.1);
        let (lx, ly) = (vx * a.0 + vy * a.1, vx * n.0 + vy * n.1);
        let mut cands: [Option<(f64, f64)>; 3] = [None; 3];
        // Arc.
        let (dx, dy) = (lx, ly - rho);
        let th = dx.atan2(-dy);
        if (0.0..=beta).contains(&th) {
            cands[0] = Some((rho * th, rho - (dx * dx + dy * dy).sqrt()));
        }
        // Rigid continuation beyond the end.
        let (ex, ey) = (lx - pe.0, ly - pe.1);
        let u2 = ex * t.0 + ey * t.1;
        if u2 >= 0.0 {
            cands[1] = Some((l + u2, ex * nn.0 + ey * nn.1));
        }
        if lx < 0.0 && static_pre {
            cands[2] = Some((lx, ly));
        }
        for (u, w) in cands.into_iter().flatten() {
            let (sx, sy) = to_src(u, w);
            let v = src.sample_bilinear(sx, sy);
            if v[3] > 1e-4 {
                return v;
            }
        }
        [0.0; 4]
    });
    b
}

// ---------------------------------------------------------------- CC Lens

fn cc_lens(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = b.to_px(ctx.params.v2("center"));
    let (_, _, lw, lh) = layer_rect(ctx, &b);
    let r = (ctx.params.f("size") / 100.0 * lw.max(lh) * 0.5).max(0.5);
    let k = 2f64.powf(ctx.params.f("convergence") / 50.0);
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let d = (dx * dx + dy * dy).sqrt();
        if d >= r {
            return None;
        }
        if d < 1e-9 {
            return Some((x, y));
        }
        let g = ((d / r).asin() / FRAC_PI_2).powf(k);
        let s = r * g / d;
        Some((c.0 + dx * s, c.1 + dy * s))
    });
    b
}

// ---------------------------------------------------------------- CC Page Turn

fn page_turn(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let f = b.to_px(ctx.params.v2("foldPosition"));
    let phi = ctx.params.f("foldDirection").to_radians();
    let r = (ctx.params.f("foldRadius") * b.scale).max(0.5);
    let lam = ctx.params.f("lightDirection").to_radians();
    let render = ctx.params.e("render");
    let bo = (ctx.params.f("backPageOpacity") / 100.0) as f32;
    let paper = ctx.params.color("paperColor");
    // The fold travels along `fd`; the lifted part lies on the other side (toward `n`).
    let n = (-phi.sin(), phi.cos());
    let ldot = (n.0 * lam.sin() - n.1 * lam.cos()) as f32;
    let (show_front, show_back) = (render != 1, render != 2);
    let src = b.img.clone();
    map_xy(&mut b.img, |x, y, _| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let d = (px - f.0) * n.0 + (py - f.1) * n.1;
        if d > r {
            return [0.0; 4];
        }
        // (u, back side?, shade) from the top layer down.
        let mut cands: [Option<(f64, bool, f32)>; 4] = [None; 4];
        if d <= 0.0 {
            cands[0] = Some((PI * r - d, true, 0.9));
            cands[3] = Some((d, false, 1.0));
        } else {
            let a = (d / r).clamp(-1.0, 1.0).asin();
            cands[1] = Some((PI * r - r * a, true, 0.6 + 0.4 * a.cos() as f32));
            let psi = (a / FRAC_PI_2) as f32;
            cands[2] = Some((r * a, false, 1.0 - 0.4 * psi * (0.5 + 0.5 * ldot)));
        }
        for (u, back, shade) in cands.into_iter().flatten() {
            if (back && !show_back) || (!back && !show_front) {
                continue;
            }
            let k = u - d;
            let v = src.sample_bilinear(px + k * n.0, py + k * n.1);
            if v[3] <= 1e-4 {
                continue;
            }
            let (c, a) = unpremul(v);
            let c = if back { [0, 1, 2].map(|i| paper[i] * (1.0 - bo) + c[i] * bo) } else { c };
            return premul(c.map(|v| v * shade), a);
        }
        [0.0; 4]
    });
    b
}

// ---------------------------------------------------------------- CC Tiler / CC Griddler

fn cc_tiler(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let s = (ctx.params.f("scale") / 100.0).max(0.001);
    let c = b.to_px(ctx.params.v2("center"));
    let blend = (ctx.params.f("blendWithOriginal") / 100.0) as f32;
    let (x0, y0, w, h) = layer_rect(ctx, &b);
    let (w, h) = (w.max(1.0), h.max(1.0));
    let src = b.img.clone();
    map_xy(&mut b.img, |x, y, px| {
        let qx = c.0 + (x as f64 + 0.5 - c.0) / s;
        let qy = c.1 + (y as f64 + 0.5 - c.1) / s;
        let sx = x0 + (qx - x0).rem_euclid(w);
        let sy = y0 + (qy - y0).rem_euclid(h);
        let t: Px = src.sample_bilinear_clamped(sx, sy);
        lerp4(t, px, blend)
    });
    b
}

fn cc_griddler(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let hs = (ctx.params.f("horizontalScale") / 100.0).max(0.001);
    let vs = (ctx.params.f("verticalScale") / 100.0).max(0.001);
    let rot = ctx.params.f("rotation").to_radians();
    let cut = ctx.params.b("cutTiles");
    let (x0, y0, lw, _) = layer_rect(ctx, &b);
    let t = (ctx.params.f("tileSize") / 100.0 * lw).max(1.0);
    if (hs - 1.0).abs() < 1e-9 && (vs - 1.0).abs() < 1e-9 && rot.abs() < 1e-9 && !cut {
        return b;
    }
    let (sr, cr) = rot.sin_cos();
    let src = b.img.clone();
    map_xy(&mut b.img, |x, y, _| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let tcx = x0 + (((px - x0) / t).floor() + 0.5) * t;
        let tcy = y0 + (((py - y0) / t).floor() + 0.5) * t;
        let (lx, ly) = (px - tcx, py - tcy);
        if cut && lx.abs().max(ly.abs()) > 0.45 * t {
            return [0.0; 4];
        }
        let (rx, ry) = (lx * cr + ly * sr, -lx * sr + ly * cr);
        src.sample_bilinear(tcx + rx / hs, tcy + ry / vs)
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let srcs = || popup(&SRC_NAMES);
    vec![
        spec(
            "ec.distort.turbulentdisplace",
            "Turbulent Displace",
            vec![
                p("displacement", "Displacement", Value::Enum(0), popup(&TURBULENT_KINDS)),
                p("amount", "Amount", num(50.0), slider(-1000.0, 1000.0, -200.0, 200.0, 1)),
                p("size", "Size", num(100.0), slider(2.0, 4000.0, 2.0, 400.0, 1)),
                p("offset", "Offset (Turbulence)", pt(0.5, 0.5), ParamUi::Point),
                p("complexity", "Complexity", num(1.0), slider(1.0, 10.0, 1.0, 10.0, 1)),
                p("evolution", "Evolution", num(0.0), ParamUi::Angle),
                p("evolutionOptions/cycleEvolution", "Cycle Evolution", Value::Bool(false), ParamUi::Checkbox),
                p("evolutionOptions/cycle", "Cycle (in Revolutions)", num(1.0), slider(1.0, 1000.0, 1.0, 20.0, 0)),
                p("evolutionOptions/randomSeed", "Random Seed", num(0.0), slider(0.0, 10000.0, 0.0, 1000.0, 0)),
                p("pinning", "Pinning", Value::Enum(1), popup(&TURBULENT_PINNING)),
                p("resizeLayer", "Resize Layer", Value::Bool(false), ParamUi::Checkbox),
            ],
            turbulent_displace,
        ),
        spec(
            "ec.distort.displacementmap",
            "Displacement Map",
            vec![
                p("displacementMapLayer", "Displacement Map Layer", Value::Layer(None), ParamUi::Layer),
                // The map layer is read without its masks or effects.
                p("displacementMapLayerSource", "Displacement Map Layer Source", Value::Enum(0), ParamUi::Hidden),
                p("useForHorizontal", "Use For Horizontal Displacement", Value::Enum(0), srcs()),
                p("maxHorizontal", "Max Horizontal Displacement", num(5.0), slider(-32000.0, 32000.0, -100.0, 100.0, 1)),
                p("useForVertical", "Use For Vertical Displacement", Value::Enum(1), srcs()),
                p("maxVertical", "Max Vertical Displacement", num(5.0), slider(-32000.0, 32000.0, -100.0, 100.0, 1)),
                p("displacementMapBehavior", "Displacement Map Behavior", Value::Enum(0), popup(&["Center Map", "Stretch Map to Fit", "Tile Map"])),
                p("wrapPixelsAround", "Wrap Pixels Around", Value::Bool(false), ParamUi::Checkbox),
                p("expandOutput", "Expand Output", Value::Bool(true), ParamUi::Checkbox),
            ],
            displacement_map,
        ),
        spec(
            "ec.distort.opticscompensation",
            "Optics Compensation",
            vec![
                p("fieldOfView", "Field Of View (FOV)", num(0.0), slider(0.0, 180.0, 0.0, 180.0, 1)),
                p("reverseLensDistortion", "Reverse Lens Distortion", Value::Bool(false), ParamUi::Checkbox),
                p("fovOrientation", "FOV Orientation", Value::Enum(0), popup(&["Horizontal", "Vertical", "Diagonal"])),
                p("viewCenter", "View Center", pt(0.5, 0.5), ParamUi::Point),
                p("optimalPixels", "Optimal Pixels (Invalidates Resize)", Value::Bool(false), ParamUi::Checkbox),
                p("resize", "Resize", Value::Enum(0), popup(&["Off", "Max 2X", "Max 4X", "Unlimited"])),
            ],
            optics_compensation,
        ),
        spec(
            "ec.distort.magnify",
            "Magnify",
            vec![
                p("shape", "Shape", Value::Enum(0), popup(&["Circle", "Square"])),
                p("center", "Center", pt(0.5, 0.5), ParamUi::Point),
                p("magnification", "Magnification", num(200.0), slider(1.0, 10000.0, 100.0, 600.0, 1)),
                p("link", "Link", Value::Enum(0), popup(&["None", "Size To Magnification", "Size & Feather To Magnification"])),
                p("size", "Size", num(100.0), slider(0.0, 4000.0, 0.0, 1000.0, 1)),
                p("feather", "Feather", num(0.0), slider(0.0, 4000.0, 0.0, 200.0, 1)),
                p("opacity", "Opacity", num(100.0), pct(100.0)),
                p("scaling", "Scaling", Value::Enum(0), popup(&["Standard", "Soft", "Scatter"])),
                p("blendingMode", "Blending Mode", Value::Enum(1), popup(&MAGNIFY_MODE_NAMES)),
                p("resizeLayer", "Resize Layer", Value::Bool(false), ParamUi::Checkbox),
            ],
            magnify,
        ),
        spec(
            "ec.distort.meshwarp",
            "Mesh Warp",
            vec![
                p("rows", "Rows", num(7.0), slider(1.0, 31.0, 1.0, 31.0, 0)),
                p("columns", "Columns", num(7.0), slider(1.0, 31.0, 1.0, 31.0, 0)),
                p("quality", "Quality", num(8.0), slider(1.0, 10.0, 1.0, 10.0, 0)),
                p("mesh", "Distortion Mesh", Value::Str(String::new()), ParamUi::Text),
            ],
            mesh_warp,
        ),
        spec(
            "ec.distort.bezierwarp",
            "Bezier Warp",
            vec![
                p("topLeftVertex", "Top Left Vertex", pt(0.0, 0.0), ParamUi::Point),
                p("topLeftTangent", "Top Left Tangent", pt(1.0 / 3.0, 0.0), ParamUi::Point),
                p("topRightTangent", "Top Right Tangent", pt(2.0 / 3.0, 0.0), ParamUi::Point),
                p("rightTopVertex", "Right Top Vertex", pt(1.0, 0.0), ParamUi::Point),
                p("rightTopTangent", "Right Top Tangent", pt(1.0, 1.0 / 3.0), ParamUi::Point),
                p("rightBottomTangent", "Right Bottom Tangent", pt(1.0, 2.0 / 3.0), ParamUi::Point),
                p("bottomRightVertex", "Bottom Right Vertex", pt(1.0, 1.0), ParamUi::Point),
                p("bottomRightTangent", "Bottom Right Tangent", pt(2.0 / 3.0, 1.0), ParamUi::Point),
                p("bottomLeftTangent", "Bottom Left Tangent", pt(1.0 / 3.0, 1.0), ParamUi::Point),
                p("leftBottomVertex", "Left Bottom Vertex", pt(0.0, 1.0), ParamUi::Point),
                p("leftBottomTangent", "Left Bottom Tangent", pt(0.0, 2.0 / 3.0), ParamUi::Point),
                p("leftTopTangent", "Left Top Tangent", pt(0.0, 1.0 / 3.0), ParamUi::Point),
                p("quality", "Quality", num(8.0), slider(1.0, 10.0, 1.0, 10.0, 0)),
            ],
            bezier_warp,
        ),
        spec(
            "ec.distort.ccbendit",
            "CC Bend It",
            vec![
                p("bend", "Bend", num(0.0), ParamUi::Angle),
                p("start", "Start", pt(0.5, 0.2), ParamUi::Point),
                p("end", "End", pt(0.5, 0.8), ParamUi::Point),
                p("renderPrestart", "Render Prestart", Value::Enum(1), popup(&["None", "Static"])),
            ],
            bend_it,
        ),
        spec(
            "ec.distort.cclens",
            "CC Lens",
            vec![
                p("center", "Center", pt(0.5, 0.5), ParamUi::Point),
                p("size", "Size", num(50.0), slider(0.0, 400.0, 0.0, 200.0, 1)),
                p("convergence", "Convergence", num(0.0), slider(-100.0, 100.0, -100.0, 100.0, 1)),
            ],
            cc_lens,
        ),
        spec(
            "ec.distort.ccpageturn",
            "CC Page Turn",
            vec![
                p("foldPosition", "Fold Position", pt(0.7, 0.7), ParamUi::Point),
                p("foldDirection", "Fold Direction", num(315.0), ParamUi::Angle),
                p("foldRadius", "Fold Radius", num(20.0), slider(0.0, 1000.0, 0.0, 200.0, 1)),
                p("lightDirection", "Light Direction", num(-45.0), ParamUi::Angle),
                p("render", "Render", Value::Enum(0), popup(&["Front & Back Page", "Back Page", "Front Page"])),
                p("backPageOpacity", "Back Opacity", num(75.0), pct(100.0)),
                p("paperColor", "Paper Color", col(1.0, 1.0, 1.0), ParamUi::Color),
            ],
            page_turn,
        ),
        spec(
            "ec.distort.cctiler",
            "CC Tiler",
            vec![
                p("scale", "Scale", num(50.0), slider(1.0, 10000.0, 1.0, 200.0, 1)),
                p("center", "Center", pt(0.5, 0.5), ParamUi::Point),
                p("blendWithOriginal", "Blend w. Original", num(0.0), pct(100.0)),
            ],
            cc_tiler,
        ),
        spec(
            "ec.distort.ccgriddler",
            "CC Griddler",
            vec![
                p("horizontalScale", "Horizontal Scale", num(100.0), slider(1.0, 1000.0, 1.0, 400.0, 1)),
                p("verticalScale", "Vertical Scale", num(100.0), slider(1.0, 1000.0, 1.0, 400.0, 1)),
                p("tileSize", "Tile Size", num(10.0), slider(0.5, 100.0, 0.5, 100.0, 1)),
                p("rotation", "Rotation", num(0.0), ParamUi::Angle),
                p("cutTiles", "Cut Tiles", Value::Bool(false), ParamUi::Checkbox),
            ],
            cc_griddler,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Params;

    fn test_img(w: u32, h: u32) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let a = 1.0;
                img.set(x, y, [x as f32 / w as f32 * a, y as f32 / h as f32 * a, ((x * 7 + y * 3) % 11) as f32 / 11.0 * a, a]);
            }
        }
        img
    }

    fn run(id: &str, set: &[(&str, Value)], img: Image) -> Buf {
        let s = crate::find(id).unwrap();
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        let ls = [img.width as f64, img.height as f64];
        for p in &s.params {
            if let (ParamUi::Point, Value::Vec2(v)) = (&p.ui, &p.default) {
                params.values.insert(p.id.to_string(), Value::Vec2([v[0] * ls[0], v[1] * ls[1]]));
            }
        }
        for (k, v) in set {
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx = EffectCtx { params: &params, time: 0.5, layer_size: ls, seed: 3, adjustment: false, env: Default::default() };
        crate::apply(s, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 })
    }

    fn max_diff(a: &Image, b: &Image) -> f32 {
        a.data.iter().zip(&b.data).map(|(p, q)| (0..4).map(|i| (p[i] - q[i]).abs()).fold(0.0, f32::max)).fold(0.0, f32::max)
    }

    #[test]
    fn displacement_map_half_is_identity() {
        let img = test_img(32, 24);
        let o = run(
            "ec.distort.displacementmap",
            &[("useForHorizontal", Value::Enum(9)), ("useForVertical", Value::Enum(9)), ("expandOutput", Value::Bool(false))],
            img.clone(),
        );
        assert_eq!(o.img, img);
        let o = run(
            "ec.distort.displacementmap",
            &[("useForHorizontal", Value::Enum(8)), ("useForVertical", Value::Enum(10)), ("maxHorizontal", num(2.0)), ("expandOutput", Value::Bool(false))],
            img.clone(),
        );
        // Full → +2 px horizontally: pixel x shows source x + 2; Off → no vertical displacement.
        let (a, b2) = (o.img.get(5, 5), img.get(7, 5));
        assert!((0..4).all(|i| (a[i] - b2[i]).abs() < 1e-5), "{a:?} vs {b2:?}");
    }

    #[test]
    fn grid_bins_bound_pathological_mesh_indexing() {
        // All maximum-grid triangles span the whole image: the previous per-row index
        // would allocate over 23 GiB at the largest height, before the output pixels themselves.
        let triangles = vec![[0, 1, 2]; 31 * 31 * 10 * 10 * 2];
        let dest = [(0.0, -1e6), (1.0, 1e6), (2.0, -1e6)];
        for height in [0, 1, 2160, 16384] {
            let bins = grid_bins(&triangles, &dest, height);
            let entries: usize = bins.bins.iter().map(Vec::len).sum();
            assert!(entries <= GRID_BIN_INDEX_BUDGET, "{height}: {entries} entries");
            assert_eq!(bins.bins.iter().map(Vec::capacity).sum::<usize>(), entries);
            if height != 0 {
                for row in [0, height / 2, height - 1] {
                    assert_eq!(bins.bins[row / bins.rows_per_bin].len(), triangles.len());
                }
            }
        }
    }

    #[test]
    fn grid_bins_preserve_row_candidates_and_triangle_order() {
        let dest = [(0.0, -2.0), (1.0, 1.0), (2.0, 10.0), (3.0, 66.0), (4.0, 100.0), (5.0, 200.0), (6.0, 201.0)];
        let triangles = [[0, 1, 2], [1, 2, 3], [2, 3, 4], [4, 5, 6], [5, 6, 5], [0, 0, 1]];
        let height = 123;
        let bins = grid_bins(&triangles, &dest, height);
        assert!(bins.rows_per_bin > 1, "exercise coarsening");
        for row in 0..height {
            let expected: Vec<_> = triangles
                .iter()
                .enumerate()
                .filter_map(|(t, tri)| {
                    let ys = tri.map(|k| dest[k].1);
                    let first = (ys[0].min(ys[1]).min(ys[2]) - 0.5).floor().max(0.0) as usize;
                    let end = ((ys[0].max(ys[1]).max(ys[2]) + 0.5).ceil().max(0.0) as usize).min(height);
                    (first..end).contains(&row).then_some(t)
                })
                .collect();
            let actual: Vec<_> = bins.bins[row / bins.rows_per_bin].iter().copied().filter(|t| bins.triangle_rows[*t].contains(&row)).collect();
            assert_eq!(actual, expected, "row {row}: raster traversal is unchanged");
        }
    }

    #[test]
    fn mesh_warp_empty_and_zero_are_identity() {
        let img = test_img(32, 24);
        assert_eq!(run("ec.distort.meshwarp", &[], img.clone()).img, img);
        let zeros = vec!["0,0"; 64].join(" ");
        assert_eq!(run("ec.distort.meshwarp", &[("mesh", Value::Str(zeros))], img.clone()).img, img);
    }

    #[test]
    fn mesh_warp_uniform_offset_translates() {
        let img = test_img(32, 24);
        let shift = vec!["4,0"; 64].join(" ");
        let o = run("ec.distort.meshwarp", &[("mesh", Value::Str(shift))], img.clone());
        for y in 2..22 {
            for x in 6..30 {
                let (a, b2) = (o.img.get(x, y), img.get(x - 4, y));
                assert!((0..4).all(|i| (a[i] - b2[i]).abs() < 1e-4), "({x},{y}) {a:?} vs {b2:?}");
            }
        }
        assert!(o.img.get(1, 10)[3] < 1e-6);
    }

    #[test]
    fn mesh_warp_centered_layer_uses_the_same_bounds_as_the_viewer() {
        let img = test_img(32, 24);
        let spec = crate::find("ec.distort.meshwarp").unwrap();
        let mut params = Params { values: spec.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        let mesh = vec!["4,0"; 64].join(" ");
        params.values.insert("mesh".into(), Value::Str(mesh.clone()));
        let env = crate::EffectEnv { bounds_origin: [-16.0, -12.0], ..Default::default() };
        let ctx = EffectCtx { params: &params, time: 0.5, layer_size: [32.0, 24.0], seed: 3, adjustment: false, env };
        let centered = crate::apply(spec, &ctx, Buf { img: img.clone(), offset: [16.0, 12.0], scale: 1.0 });
        let regular = run("ec.distort.meshwarp", &[("mesh", Value::Str(mesh))], img);
        assert!(max_diff(&centered.img, &regular.img) < 1e-5);
    }

    #[test]
    fn mesh_warp_tangents_curve_the_grid_and_image() {
        let mut mesh = WarpMesh::identity(1, 1);
        mesh.controls[0][2] = [0.0, 12.0];
        assert_eq!(mesh.point(0, 1, [32.0, 24.0]), None);
        let middle = mesh.sample(0.5, 0.0, [32.0, 24.0]);
        assert!((middle[0] - 16.0).abs() < 1e-10 && (middle[1] - 4.5).abs() < 1e-10);
        assert_eq!(WarpMesh::parse(&mesh.format(), 1, 1), mesh);
        let img = test_img(32, 24);
        for quality in [1.0, 8.0] {
            let out = run(
                "ec.distort.meshwarp",
                &[("rows", num(1.0)), ("columns", num(1.0)), ("quality", num(quality)), ("mesh", Value::Str(mesh.format()))],
                img.clone(),
            );
            assert!(out.img.get(16, 1)[3] < 1e-6, "Quality {quality}: the bent top edge must reveal transparency");
            assert!(max_diff(&out.img, &img) > 0.1);
        }
        // Opposing tangent adjustments cancel at t=0, 0.5 and 1. Sampling only those
        // positions loses this S-curve completely; Quality 1 must still show its deformation.
        mesh.controls[1][1] = [0.0, -12.0];
        assert!(mesh.sample(0.5, 0.0, [32.0, 24.0])[1].abs() < 1e-10);
        assert!(mesh.sample(1.0 / 3.0, 0.0, [32.0, 24.0])[1] > 2.0);
        for quality in [1.0, 8.0] {
            let out = run(
                "ec.distort.meshwarp",
                &[("rows", num(1.0)), ("columns", num(1.0)), ("quality", num(quality)), ("mesh", Value::Str(mesh.format()))],
                img.clone(),
            );
            assert!(out.img.get(10, 1)[3] < 1e-6, "Quality {quality}: opposing tangents must curve the top edge");
            assert!(max_diff(&out.img, &img) > 0.1);
        }
    }

    #[test]
    fn mesh_warp_malformed_data_is_bounded_and_repairs_to_identity() {
        for text in ["mw1;99999999999999999999,1;", "mw1;0,0;", "mw1;1,1;NaN,0", "inf,0 0,0 0,0 0,0", "1,0"] {
            assert!(WarpMesh::parse(text, 1, 1).is_identity(), "{text}");
        }
        assert_eq!(parse_mesh(&vec!["1,1"; 4096].join(" ")).len(), 1025);
        assert_eq!(WarpMesh::identity(usize::MAX, usize::MAX).controls.len(), 1024);
        assert_eq!(WarpMesh::dimension(f64::NAN), 7);
        let mut mesh = WarpMesh::identity(1, 1);
        for vertex in &mut mesh.controls {
            vertex[0] = [5.0, -2.0];
        }
        let resized = WarpMesh::parse(&mesh.format(), 3, 2);
        assert!(resized.controls.iter().all(|v| v[0] == [5.0, -2.0]));
    }

    #[test]
    fn mesh_warp_keyframes_interpolate_vertices_and_tangents_and_honor_hold() {
        let mut mesh = WarpMesh::identity(1, 1);
        mesh.controls[0][0] = [20.0, 10.0];
        mesh.controls[0][2] = [0.0, 12.0];
        let mut prop = Property::new(1, "mesh", "Distortion Mesh", Value::Str(String::new()));
        prop.keys = vec![Keyframe::new(Tick::ZERO, Value::Str(String::new())), Keyframe::new(Tick::from_seconds_f64(1.0), Value::Str(mesh.format()))];
        let value = mesh_value(&prop, 1, 1, Tick::from_seconds_f64(0.5));
        let Value::Str(text) = value else { panic!("expected mesh data") };
        let middle = WarpMesh::parse(&text, 1, 1);
        assert_eq!(middle.controls[0][0], [10.0, 5.0]);
        assert_eq!(middle.controls[0][2], [0.0, 6.0]);
        effectcraft_keyframe::easy_ease(&mut prop.keys[0], false, true);
        effectcraft_keyframe::easy_ease(&mut prop.keys[1], true, false);
        let Value::Str(eased) = mesh_value(&prop, 1, 1, Tick::from_seconds_f64(0.25)) else { panic!("expected mesh data") };
        assert!(WarpMesh::parse(&eased, 1, 1).controls[0][0][0] < 5.0);
        prop.keys[0].out_interp = effectcraft_keyframe::Interp::Hold;
        let Value::Str(held) = mesh_value(&prop, 1, 1, Tick::from_seconds_f64(0.5)) else { panic!("expected mesh data") };
        assert!(WarpMesh::parse(&held, 1, 1).is_identity());
    }

    #[test]
    fn bezier_warp_default_is_identity() {
        let img = test_img(32, 24);
        let o = run("ec.distort.bezierwarp", &[], img.clone());
        assert!(max_diff(&o.img, &img) < 1e-3, "{}", max_diff(&o.img, &img));
    }

    #[test]
    fn optics_fov_zero_is_identity_and_nonzero_warps() {
        let img = test_img(32, 24);
        assert_eq!(run("ec.distort.opticscompensation", &[], img.clone()).img, img);
        assert!(max_diff(&run("ec.distort.opticscompensation", &[("fieldOfView", num(90.0))], img.clone()).img, &img) > 1e-3);
    }

    #[test]
    fn bend_it_zero_is_identity() {
        let img = test_img(32, 24);
        assert_eq!(run("ec.distort.ccbendit", &[], img.clone()).img, img);
        assert!(max_diff(&run("ec.distort.ccbendit", &[("bend", num(45.0))], img.clone()).img, &img) > 1e-3);
    }

    #[test]
    fn magnify_100_is_identity() {
        let img = test_img(32, 24);
        assert!(max_diff(&run("ec.distort.magnify", &[("magnification", num(100.0))], img.clone()).img, &img) < 1e-6);
        let o = run("ec.distort.magnify", &[("size", num(8.0))], img.clone());
        // Centre pixel unchanged; slightly off-centre samples move toward the centre.
        assert!(max_diff(&o.img, &img) > 1e-3);
        assert_eq!(o.img.get(0, 0), img.get(0, 0));
    }

    #[test]
    fn tiler_100_is_identity_and_50_tiles() {
        let img = test_img(32, 24);
        assert!(max_diff(&run("ec.distort.cctiler", &[("scale", num(100.0))], img.clone()).img, &img) < 1e-6);
        let o = run("ec.distort.cctiler", &[], img.clone());
        assert!(o.img.data.iter().all(|p| p[3] > 0.99));
    }

    #[test]
    fn turbulent_displace_zero_amount_is_identity() {
        let img = test_img(32, 24);
        assert_eq!(run("ec.distort.turbulentdisplace", &[("amount", num(0.0))], img.clone()).img, img);
        assert!(max_diff(&run("ec.distort.turbulentdisplace", &[], img.clone()).img, &img) > 1e-3);
    }

    #[test]
    fn griddler_default_is_identity() {
        let img = test_img(32, 24);
        assert_eq!(run("ec.distort.ccgriddler", &[], img.clone()).img, img);
    }

    #[test]
    fn page_turn_removes_corner_and_keeps_far_side() {
        let img = test_img(64, 64);
        let o = run("ec.distort.ccpageturn", &[("foldRadius", num(4.0))], img.clone());
        // Bottom-right corner is lifted away; top-left untouched.
        assert!(o.img.get(63, 63)[3] < 1e-6);
        assert_eq!(o.img.get(2, 2), img.get(2, 2));
    }

    #[test]
    fn cc_lens_clears_outside() {
        let img = test_img(32, 32);
        let o = run("ec.distort.cclens", &[], img);
        assert!(o.img.get(0, 0)[3] < 1e-6);
        assert!(o.img.get(16, 16)[3] > 0.99);
    }

    /// A host whose every layer is a flat 8×8 image of `px`.
    struct FlatHost([f32; 4]);
    impl crate::EffectHost for FlatHost {
        fn layer(&self, _: u64, _: bool) -> Option<crate::LayerPixels> {
            Some(crate::LayerPixels { buf: Buf { img: Image::filled(8, 8, self.0), offset: [0.0; 2], scale: 1.0 }, size: [8.0, 8.0] })
        }
        fn audio(&self, _: u64, _: f64, _: usize, _: u32) -> Option<Vec<f32>> {
            None
        }
    }

    #[test]
    fn displacement_map_reads_the_map_layer_and_expands_output() {
        let img = test_img(32, 24);
        // A white map layer → Red = 1 → full positive horizontal displacement everywhere.
        let host = FlatHost([1.0, 1.0, 1.0, 1.0]);
        let s = crate::find("ec.distort.displacementmap").unwrap();
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        for (k, v) in [
            ("displacementMapLayer", Value::Layer(Some(7))),
            ("useForVertical", Value::Enum(10)),
            ("maxHorizontal", num(3.0)),
            ("expandOutput", Value::Bool(false)),
        ] {
            params.values.insert(k.into(), v);
        }
        for behavior in 0..3 {
            params.values.insert("displacementMapBehavior".into(), Value::Enum(behavior));
            let env = crate::EffectEnv { host: Some(&host), ..Default::default() };
            let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [32.0, 24.0], seed: 1, adjustment: false, env };
            let o = crate::apply(s, &ctx, Buf { img: img.clone(), offset: [0.0, 0.0], scale: 1.0 });
            if behavior == 0 {
                // Center Map: the 8×8 map covers only the middle; outside it nothing moves.
                assert_eq!(o.img.get(2, 2), img.get(2, 2));
                assert!((o.img.get(16, 12)[0] - img.get(19, 12)[0]).abs() < 1e-5);
            } else {
                // Stretched or tiled, the map covers the whole layer.
                assert!(
                    (o.img.get(2, 2)[0] - img.get(5, 2)[0]).abs() < 1e-5,
                    "behavior {behavior}: {:?} {:?} {:?}",
                    o.img.get(2, 2),
                    img.get(5, 2),
                    img.get(2, 2)
                );
            }
        }
        // Expand Output grows the layer by the maximum displacement.
        let o = run("ec.distort.displacementmap", &[("maxHorizontal", num(6.0))], img.clone());
        assert_eq!((o.img.width, o.img.height), (32 + 12, 24 + 12));
    }

    #[test]
    fn turbulent_displace_pinning_and_cycle_evolution() {
        let img = test_img(40, 40);
        let pinned = run("ec.distort.turbulentdisplace", &[("size", num(10.0))], img.clone());
        let free = run("ec.distort.turbulentdisplace", &[("size", num(10.0)), ("pinning", Value::Enum(0))], img.clone());
        // Pin All (the default) holds the border.
        for i in 0..40 {
            assert_eq!(pinned.img.get(i, 0), img.get(i, 0));
            assert_eq!(pinned.img.get(0, i), img.get(0, i));
        }
        assert!(max_diff(&pinned.img, &free.img) > 1e-3);
        // Cycle Evolution: evolution 0 and `cycle` revolutions render the same frame.
        let cyc = |evo: f64| {
            run(
                "ec.distort.turbulentdisplace",
                &[("evolution", num(evo)), ("evolutionOptions/cycleEvolution", Value::Bool(true)), ("evolutionOptions/cycle", num(2.0))],
                img.clone(),
            )
            .img
        };
        assert!(max_diff(&cyc(0.0), &cyc(720.0)) < 1e-4);
        assert!(max_diff(&cyc(0.0), &cyc(200.0)) > 1e-3);
        // Random Seed changes the pattern.
        let seeded = run("ec.distort.turbulentdisplace", &[("size", num(10.0)), ("evolutionOptions/randomSeed", num(5.0))], img);
        assert!(max_diff(&seeded.img, &pinned.img) > 1e-3);
        // Locked pinning: no sample comes from outside the layer (no transparent pixels pulled
        // in) and the border still holds.
        let opaque = Image::filled(40, 40, [0.5, 0.5, 0.5, 1.0]);
        let strong = [("size", num(10.0)), ("amount", num(80.0))];
        let left = run("ec.distort.turbulentdisplace", &[strong.as_slice(), &[("pinning", Value::Enum(4))]].concat(), opaque.clone());
        let locked = run("ec.distort.turbulentdisplace", &[strong.as_slice(), &[("pinning", Value::Enum(11))]].concat(), opaque.clone());
        assert!(left.img.data.iter().any(|p| p[3] < 0.99), "plain pin lets edges pull in");
        assert!(locked.img.data.iter().all(|p| p[3] > 0.999), "locked keeps everything inside");
        assert_eq!(TURBULENT_PINNING[11], "Pin Left Locked");
    }

    #[test]
    fn magnify_blending_none_and_scaling() {
        let img = test_img(32, 32);
        let none = run("ec.distort.magnify", &[("size", num(6.0)), ("blendingMode", Value::Enum(0))], img.clone());
        assert!(none.img.get(0, 0)[3] < 1e-6);
        assert!(none.img.get(16, 16)[3] > 0.99);
        let std = run("ec.distort.magnify", &[("magnification", num(400.0))], img.clone());
        let soft = run("ec.distort.magnify", &[("magnification", num(400.0)), ("scaling", Value::Enum(1))], img.clone());
        assert!(max_diff(&std.img, &soft.img) > 1e-3);
        let add = run("ec.distort.magnify", &[("blendingMode", Value::Enum(2))], img);
        assert!(max_diff(&add.img, &std.img) > 1e-3);
    }

    #[test]
    fn optics_resize_grows_reversed_layers() {
        let img = test_img(32, 24);
        let set = |r: u32| vec![("fieldOfView", num(120.0)), ("reverseLensDistortion", Value::Bool(true)), ("resize", Value::Enum(r))];
        let off = run("ec.distort.opticscompensation", &set(0), img.clone());
        assert_eq!(off.img.width, 32);
        let max2 = run("ec.distort.opticscompensation", &set(1), img.clone());
        assert!(max2.img.width > 32 && max2.img.width <= 32 + 2 * 16 + 2, "{}", max2.img.width);
        let max4 = run("ec.distort.opticscompensation", &set(2), img);
        assert!(max4.img.width >= max2.img.width);
    }

    #[test]
    fn optics_optimal_pixels_fills_or_fits_the_layer() {
        let img = Image::filled(32, 24, [0.5, 0.5, 0.5, 1.0]);
        // Forward: without Optimal Pixels the corners sample outside the layer.
        let plain = run("ec.distort.opticscompensation", &[("fieldOfView", num(120.0))], img.clone());
        assert!(plain.img.get(0, 0)[3] < 0.5);
        let opt = run("ec.distort.opticscompensation", &[("fieldOfView", num(120.0)), ("optimalPixels", Value::Bool(true))], img.clone());
        assert!(opt.img.get(0, 0)[3] > 0.9 && opt.img.get(31, 23)[3] > 0.9);
        // Reverse: everything fits; Resize is ignored.
        let rev = [("fieldOfView", num(120.0)), ("reverseLensDistortion", Value::Bool(true)), ("optimalPixels", Value::Bool(true)), ("resize", Value::Enum(2))];
        let r = run("ec.distort.opticscompensation", &rev, img.clone());
        assert_eq!(r.img.width, 32);
        // The edges of the source now sit inside the layer: empty space around them.
        assert!(r.img.get(0, 0)[3] < 0.5 && r.img.get(16, 12)[3] > 0.9);
    }
}
