//! Pixel-level helpers: fast blends on premultiplied RGBA8, box blur,
//! deterministic noise and precomputed effect maps.

use tiny_skia::Pixmap;

/// `x / 255` with rounding, exact for `x <= 255 * 255`.
#[inline(always)]
pub(crate) fn div255(x: u32) -> u32 {
    let x = x + 128;
    (x + (x >> 8)) >> 8
}

/// Source-over of a straight-alpha colour `rgb` with coverage `a` (0..=255)
/// onto a premultiplied RGBA pixel.
#[inline(always)]
pub(crate) fn over_px(px: &mut [u8; 4], rgb: [u32; 3], a: u32) {
    let ia = 255 - a;
    let na = (a + div255(px[3] as u32 * ia)).min(255);
    px[0] = (div255(rgb[0] * a) + div255(px[0] as u32 * ia)).min(na) as u8;
    px[1] = (div255(rgb[1] * a) + div255(px[1] as u32 * ia)).min(na) as u8;
    px[2] = (div255(rgb[2] * a) + div255(px[2] as u32 * ia)).min(na) as u8;
    px[3] = na as u8;
}

/// Source-over of one colour with a uniform coverage over a run of pixels.
pub(crate) fn over_span(span: &mut [u8], rgb: [u32; 3], a: u32) {
    if a == 0 {
        return;
    }
    if rgb == [0, 0, 0] {
        let ia = 255 - a;
        for px in span.as_chunks_mut::<4>().0 {
            px[0] = div255(px[0] as u32 * ia) as u8;
            px[1] = div255(px[1] as u32 * ia) as u8;
            px[2] = div255(px[2] as u32 * ia) as u8;
            px[3] = (a + div255(px[3] as u32 * ia)).min(255) as u8;
        }
    } else {
        for px in span.as_chunks_mut::<4>().0 {
            over_px(px, rgb, a);
        }
    }
}

/// Scale every channel of a premultiplied span by `k / 256`.
pub(crate) fn scale_span(span: &mut [u8], k: u32) {
    if k >= 256 {
        return;
    }
    for v in span.iter_mut() {
        *v = ((*v as u32 * k) >> 8) as u8;
    }
}

/// SplitMix64: tiny deterministic generator for seeded randomness.
#[derive(Debug, Clone)]
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0)
    }
    /// Uniform in 0..1.
    pub(crate) fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub(crate) fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }
}

#[inline]
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Stateless hash of two integers to a u64.
#[inline]
pub(crate) fn hash2(a: u64, b: u64) -> u64 {
    mix(a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ mix(b.wrapping_add(0x632B_E59B_D9B4_E019)))
}

/// Hash to a float in -1..1.
#[inline]
pub(crate) fn hash_signed(a: u64, b: u64) -> f32 {
    (hash2(a, b) >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

/// Three passes of a box blur (≈ Gaussian) on a premultiplied pixmap.
/// `sigma` is in pixels of this pixmap.
pub(crate) fn blur_pixmap(pixmap: &mut Pixmap, sigma: f32) {
    if sigma < 0.3 {
        return;
    }
    // Three box passes of radius r approximate a Gaussian of sigma sqrt(r(r+1)).
    let r = ((sigma * sigma + 0.25).sqrt() - 0.5).round().max(1.0) as usize;
    let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
    let data = pixmap.data_mut();
    let mut tmp = vec![0u8; w.max(h) * 4];
    for _ in 0..3 {
        for y in 0..h {
            box_line(&mut data[y * w * 4..(y + 1) * w * 4], 4, w, r, &mut tmp);
        }
        for x in 0..w {
            box_line(&mut data[x * 4..], w * 4, h, r, &mut tmp);
        }
    }
}

/// Box-filter `n` pixels spaced `stride` bytes apart, in place.
fn box_line(data: &mut [u8], stride: usize, n: usize, r: usize, tmp: &mut [u8]) {
    if n == 0 {
        return;
    }
    for i in 0..n {
        tmp[i * 4..i * 4 + 4].copy_from_slice(&data[i * stride..i * stride + 4]);
    }
    let win = (2 * r + 1) as u32;
    let mut sum = [0u32; 4];
    // Edges are treated as transparent.
    for i in 0..=r.min(n - 1) {
        for c in 0..4 {
            sum[c] += tmp[i * 4 + c] as u32;
        }
    }
    for i in 0..n {
        for c in 0..4 {
            data[i * stride + c] = ((sum[c] + win / 2) / win) as u8;
        }
        let add = i + r + 1;
        if add < n {
            for c in 0..4 {
                sum[c] += tmp[add * 4 + c] as u32;
            }
        }
        if i >= r {
            let sub = i - r;
            for c in 0..4 {
                sum[c] -= tmp[sub * 4 + c] as u32;
            }
        }
    }
}

/// Smooth Hermite step.
#[inline]
pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Precomputed vignette coverage (0..=255) for every canvas pixel, dithered
/// to avoid banding in the dark corners.
pub(crate) fn vignette_map(w: u32, h: u32, strength: f32, radius: f32, softness: f32) -> Vec<u8> {
    let (wf, hf) = (w as f32, h as f32);
    let (cx, cy) = (wf / 2.0, hf / 2.0);
    let mut map = vec![0u8; (w * h) as usize];
    let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
    for y in 0..h {
        let ny = (y as f32 + 0.5 - cy) / cy;
        let row = &mut map[(y * w) as usize..((y + 1) * w) as usize];
        for (x, v) in row.iter_mut().enumerate() {
            let nx = (x as f32 + 0.5 - cx) / cx;
            let d = (nx * nx + ny * ny).sqrt() * inv_sqrt2;
            let a = smoothstep(radius, radius + softness, d);
            if a <= 0.0 {
                continue;
            }
            let dither = hash_signed(x as u64, y as u64) * 0.5;
            *v = (a * a * strength * 255.0 + dither)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
    map
}

pub(crate) const GRAIN_TILE: usize = 256;
pub(crate) const GRAIN_TILES: usize = 4;

/// Signed noise tiles (roughly Gaussian, -127..127), with every noise cell
/// expanded to `cell` x `cell` pixels so rows can be read contiguously.
/// Returns the tiles and their side length in pixels.
pub(crate) fn grain_tiles(seed: u64, cell: usize) -> (Vec<i8>, usize) {
    let mut rng = Rng::new(seed);
    let cells: Vec<i8> = (0..GRAIN_TILE * GRAIN_TILE * GRAIN_TILES)
        .map(|_| {
            let g = (rng.f32() + rng.f32() + rng.f32() + rng.f32() - 2.0) * 1.7;
            (g.clamp(-1.0, 1.0) * 127.0) as i8
        })
        .collect();
    let side = GRAIN_TILE * cell;
    let mut tiles = vec![0i8; side * side * GRAIN_TILES];
    for t in 0..GRAIN_TILES {
        for y in 0..side {
            let src = &cells[(t * GRAIN_TILE + y / cell) * GRAIN_TILE..][..GRAIN_TILE];
            let dst = &mut tiles[(t * side + y) * side..][..side];
            for (x, v) in dst.iter_mut().enumerate() {
                *v = src[x / cell];
            }
        }
    }
    (tiles, side)
}

/// Grain on one span: positive noise lightens towards white, negative
/// darkens towards black. `k` (0..=128) scales the noise strength.
#[inline]
fn grain_span(dst: &mut [u8], pat: &[i8], k: i16) {
    for (px, &v) in dst.as_chunks_mut::<4>().0.iter_mut().zip(pat) {
        // 7-bit coverage keeps the products inside i16 (vectorises well).
        let a = ((v as i16).abs() * k) >> 7;
        let target: i16 = if v > 0 { 255 } else { 0 };
        let r = px[0] as i16;
        let g = px[1] as i16;
        let b = px[2] as i16;
        let al = px[3] as i16;
        px[0] = (r + (((target - r) * a) >> 7)) as u8;
        px[1] = (g + (((target - g) * a) >> 7)) as u8;
        px[2] = (b + (((target - b) * a) >> 7)) as u8;
        px[3] = (al + (((255 - al) * a) >> 7)) as u8;
    }
}

/// Add film grain to the whole canvas from tile `tile` at offset (`ox`, `oy`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_grain(
    data: &mut [u8],
    w: usize,
    tiles: &[i8],
    side: usize,
    tile: usize,
    ox: usize,
    oy: usize,
    amount: f32,
) {
    // Noise values reach 127, so k = 128 * amount gives coverage up to `amount`.
    let k = (amount.clamp(0.0, 1.0) * 128.0).round() as i16;
    if k == 0 {
        return;
    }
    let tile = &tiles[tile * side * side..(tile + 1) * side * side];
    for (y, row) in data.chunks_exact_mut(w * 4).enumerate() {
        let trow = &tile[((y + oy) % side) * side..][..side];
        let mut x = 0;
        let mut sx = ox % side;
        while x < w {
            let n = (w - x).min(side - sx);
            grain_span(&mut row[x * 4..(x + n) * 4], &trow[sx..sx + n], k);
            x += n;
            sx = 0;
        }
    }
}

/// Darken by a per-pixel mask (black source-over), with `k` (0..=256)
/// scaling the mask.
pub(crate) fn darken_by_mask(data: &mut [u8], mask: &[u8], k: u16) {
    for (px, &m) in data.as_chunks_mut::<4>().0.iter_mut().zip(mask) {
        // a in 0..=256 so that x * (256 - a) >> 8 stays within u16.
        let a = ((m as u16 + (m as u16 >> 7)) * k) >> 8;
        let ia = 256 - a;
        px[0] = ((px[0] as u16 * ia) >> 8) as u8;
        px[1] = ((px[1] as u16 * ia) >> 8) as u8;
        px[2] = ((px[2] as u16 * ia) >> 8) as u8;
        let al = px[3] as u16;
        px[3] = (al + (((255 - al) * a) >> 8)) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_keeps_premultiplied_valid() {
        for a in [0u32, 1, 17, 128, 254, 255] {
            for p in [
                [0u8, 0, 0, 0],
                [10, 20, 30, 40],
                [255, 255, 255, 255],
                [0, 0, 0, 255],
            ] {
                let mut px = p;
                over_px(&mut px, [255, 200, 3], a);
                assert!(
                    px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3],
                    "{a} {p:?} -> {px:?}"
                );
            }
        }
    }

    #[test]
    fn blur_spreads_and_preserves_energy() {
        let mut p = Pixmap::new(41, 41).unwrap();
        p.data_mut()[(20 * 41 + 20) * 4..(20 * 41 + 20) * 4 + 4].copy_from_slice(&[255; 4]);
        let mut big = Pixmap::new(41, 41).unwrap();
        for v in big.data_mut().iter_mut() {
            *v = 255;
        }
        blur_pixmap(&mut big, 3.0);
        assert_eq!(big.pixel(20, 20).unwrap().alpha(), 255);
        blur_pixmap(&mut p, 2.0);
        assert!(p.pixel(22, 20).unwrap().alpha() > 0);
    }
}
