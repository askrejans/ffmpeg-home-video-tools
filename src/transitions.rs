//! Transitions between two `yuv420p` frames of the same size.

use crate::frame::{BLACK, WHITE, planes, planes_mut};
use crate::project::TransitionKind;
use rayon::prelude::*;

fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Plane geometry: (width, height) for Y, U and V.
fn geometry(w: usize, h: usize) -> [(usize, usize); 3] {
    [(w, h), (w / 2, h / 2), (w / 2, h / 2)]
}

fn split(buf: &[u8], w: usize, h: usize) -> [&[u8]; 3] {
    let (y, u, v) = planes(buf, w, h);
    [y, u, v]
}

fn split_mut(buf: &mut [u8], w: usize, h: usize) -> [&mut [u8]; 3] {
    let (y, u, v) = planes_mut(buf, w, h);
    [y, u, v]
}

/// Linear mix of two planes with weight `t` (0 = `a`, 1 = `b`).
fn mix_plane(a: &[u8], b: &[u8], out: &mut [u8], t: f32) {
    let wt = (t.clamp(0.0, 1.0) * 256.0).round() as u32;
    out.par_chunks_mut(4096)
        .zip(a.par_chunks(4096).zip(b.par_chunks(4096)))
        .for_each(|(o, (a, b))| {
            for ((o, a), b) in o.iter_mut().zip(a).zip(b) {
                *o = ((u32::from(*a) * (256 - wt) + u32::from(*b) * wt + 128) >> 8) as u8;
            }
        });
}

fn mix_value(a: &[u8], value: u8, out: &mut [u8], t: f32) {
    let wt = (t.clamp(0.0, 1.0) * 256.0).round() as u32;
    let vt = u32::from(value) * wt;
    out.par_chunks_mut(4096)
        .zip(a.par_chunks(4096))
        .for_each(|(o, a)| {
            for (o, a) in o.iter_mut().zip(a) {
                *o = ((u32::from(*a) * (256 - wt) + vt + 128) >> 8) as u8;
            }
        });
}

/// Bilinear sample of a plane scaled by `zoom` around its centre.
fn zoom_plane(src: &[u8], out: &mut [u8], w: usize, h: usize, zoom: f32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let sy = ((y as f32 + 0.5 - cy) / zoom + cy - 0.5).clamp(0.0, (h - 1) as f32);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(h - 1);
        let fy = sy - y0 as f32;
        for (x, o) in row.iter_mut().enumerate() {
            let sx = ((x as f32 + 0.5 - cx) / zoom + cx - 0.5).clamp(0.0, (w - 1) as f32);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(w - 1);
            let fx = sx - x0 as f32;
            let p = |xx: usize, yy: usize| f32::from(src[yy * w + xx]);
            let top = p(x0, y0) * (1.0 - fx) + p(x1, y0) * fx;
            let bottom = p(x0, y1) * (1.0 - fx) + p(x1, y1) * fx;
            *o = (top * (1.0 - fy) + bottom * fy).round() as u8;
        }
    });
}

/// Fast soft blur: average down by `factor`, box-blur twice, scale back up.
pub(crate) fn soft_blur(src: &[u8], w: usize, h: usize, radius: f32) -> Vec<u8> {
    if radius < 1.0 || w < 8 || h < 8 {
        return src.to_vec();
    }
    let factor = ((radius / 3.0).floor() as usize).clamp(1, 8);
    let (sw, sh) = ((w / factor).max(1), (h / factor).max(1));
    let mut small = vec![0f32; sw * sh];
    small.par_chunks_mut(sw).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let mut acc = 0u32;
            for yy in 0..factor {
                let line = &src[((y * factor + yy).min(h - 1)) * w..];
                for xx in 0..factor {
                    acc += u32::from(line[(x * factor + xx).min(w - 1)]);
                }
            }
            *o = acc as f32 / (factor * factor) as f32;
        }
    });
    let r = ((radius / factor as f32).round() as usize).max(1);
    for _ in 0..2 {
        box_pass(&mut small, sw, sh, r, true);
        box_pass(&mut small, sw, sh, r, false);
    }
    let mut out = vec![0u8; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let sy = ((y as f32 + 0.5) / factor as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let fy = sy - y0 as f32;
        for (x, o) in row.iter_mut().enumerate() {
            let sx = ((x as f32 + 0.5) / factor as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let fx = sx - x0 as f32;
            let top = small[y0 * sw + x0] * (1.0 - fx) + small[y0 * sw + x1] * fx;
            let bottom = small[y1 * sw + x0] * (1.0 - fx) + small[y1 * sw + x1] * fx;
            *o = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
        }
    });
    out
}

/// One running-sum box blur pass along rows (`horizontal`) or columns.
fn box_pass(data: &mut [f32], w: usize, h: usize, r: usize, horizontal: bool) {
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let norm = 1.0 / (2 * r + 1) as f32;
    let snapshot = data.to_vec();
    let at = |line: usize, i: usize| -> usize {
        if horizontal {
            line * w + i
        } else {
            i * w + line
        }
    };
    let results: Vec<Vec<f32>> = (0..lines)
        .into_par_iter()
        .map(|line| {
            let get = |i: isize| snapshot[at(line, i.clamp(0, len as isize - 1) as usize)];
            let mut acc: f32 = (-(r as isize)..=r as isize).map(get).sum();
            let mut out = Vec::with_capacity(len);
            for i in 0..len as isize {
                out.push(acc * norm);
                acc += get(i + r as isize + 1) - get(i - r as isize);
            }
            out
        })
        .collect();
    for (line, values) in results.into_iter().enumerate() {
        for (i, v) in values.into_iter().enumerate() {
            data[at(line, i)] = v;
        }
    }
}

/// Small deterministic hash-based noise in 0..1.
fn noise(seed: u64, a: u64, b: u64) -> f32 {
    let mut x =
        seed ^ a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// Draw a transition frame. `progress` runs from just above 0 (all `a`) to
/// just below 1 (all `b`); `frame_seed` varies per frame for noisy effects.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blend(
    kind: TransitionKind,
    a: &[u8],
    b: &[u8],
    out: &mut [u8],
    width: u32,
    height: u32,
    progress: f32,
    frame_seed: u64,
) {
    let (w, h) = (width as usize, height as usize);
    if matches!(kind, TransitionKind::Cut | TransitionKind::Mix) {
        out.copy_from_slice(if progress < 0.5 { a } else { b });
        return;
    }
    let e = ease_in_out(progress);
    let geo = geometry(w, h);
    let (pa, pb) = (split(a, w, h), split(b, w, h));
    let mut po = split_mut(out, w, h);
    match kind {
        TransitionKind::Cut | TransitionKind::Mix => unreachable!(),
        TransitionKind::Crossfade => {
            for i in 0..3 {
                mix_plane(pa[i], pb[i], po[i], e);
            }
        }
        TransitionKind::FadeBlack | TransitionKind::FadeWhite => {
            let colour = if kind == TransitionKind::FadeBlack {
                BLACK
            } else {
                WHITE
            };
            let (src, t) = if progress < 0.5 {
                (pa, ease_in_out(progress * 2.0))
            } else {
                (pb, 1.0 - ease_in_out((progress - 0.5) * 2.0))
            };
            for i in 0..3 {
                mix_value(src[i], colour[i], po[i], t);
            }
        }
        TransitionKind::Slide => {
            // The new clip pushes the old one out to the left.
            let shift = ((e * w as f32).round() as usize / 2 * 2).min(w);
            for (i, (pw, _)) in geo.iter().enumerate() {
                let s = if i == 0 { shift } else { shift / 2 };
                po[i]
                    .par_chunks_mut(*pw)
                    .zip(pa[i].par_chunks(*pw).zip(pb[i].par_chunks(*pw)))
                    .for_each(|(o, (ra, rb))| {
                        o[..pw - s].copy_from_slice(&ra[s..]);
                        o[pw - s..].copy_from_slice(&rb[..s]);
                    });
            }
        }
        TransitionKind::Wipe => {
            // A soft vertical edge reveals the new clip from left to right.
            let feather = (w as f32 / 10.0).max(2.0);
            let edge = e * (w as f32 + feather);
            for (i, (pw, _)) in geo.iter().enumerate() {
                let scale = if i == 0 { 1.0 } else { 2.0 };
                po[i]
                    .par_chunks_mut(*pw)
                    .zip(pa[i].par_chunks(*pw).zip(pb[i].par_chunks(*pw)))
                    .for_each(|(o, (ra, rb))| {
                        for x in 0..*pw {
                            let m = ((edge - x as f32 * scale) / feather).clamp(0.0, 1.0);
                            o[x] =
                                (f32::from(ra[x]) * (1.0 - m) + f32::from(rb[x]) * m).round() as u8;
                        }
                    });
            }
        }
        TransitionKind::Zoom => {
            // The old clip pushes in and dissolves into the new one settling back.
            for (i, (pw, ph)) in geo.iter().enumerate() {
                let mut za = vec![0u8; pw * ph];
                let mut zb = vec![0u8; pw * ph];
                zoom_plane(pa[i], &mut za, *pw, *ph, 1.0 + 0.3 * e);
                zoom_plane(pb[i], &mut zb, *pw, *ph, 1.15 - 0.15 * e);
                mix_plane(&za, &zb, po[i], e);
            }
        }
        TransitionKind::BlurDissolve => {
            let radius = (std::f32::consts::PI * progress).sin() * (h as f32 / 40.0);
            for (i, (pw, ph)) in geo.iter().enumerate() {
                let r = if i == 0 { radius } else { radius / 2.0 };
                let ba = soft_blur(pa[i], *pw, *ph, r);
                let bb = soft_blur(pb[i], *pw, *ph, r);
                mix_plane(&ba, &bb, po[i], e);
            }
        }
        TransitionKind::TapeRewind => {
            tape_rewind(pa, pb, &mut po, w, h, progress, frame_seed);
        }
    }
}

/// A VHS-style "rewind": the picture wobbles, a tracking band rolls through,
/// colour drains, and the next clip settles in.
fn tape_rewind(
    pa: [&[u8]; 3],
    pb: [&[u8]; 3],
    po: &mut [&mut [u8]; 3],
    w: usize,
    h: usize,
    progress: f32,
    seed: u64,
) {
    let src = if progress < 0.5 { pa } else { pb };
    let intensity = 1.0 - (2.0 * progress - 1.0).abs();
    let amp = intensity * w as f32 * 0.035;
    let band_center = ((progress * 2.3).fract()) * h as f32;
    let band_half = h as f32 * 0.05;
    let band_rows = |y: usize| -> f32 {
        let d = (y as f32 - band_center).abs();
        (1.0 - d / band_half).clamp(0.0, 1.0) * intensity
    };
    let shift_for = |y: usize| -> i32 {
        let block = (y / (h / 90).max(1)) as u64;
        let jitter = noise(seed, block, 1) * 2.0 - 1.0;
        let wave = ((y as f32 / h as f32) * 18.0 + progress * 40.0).sin() * 0.3;
        ((jitter * 0.7 + wave) * amp + band_rows(y) * amp * 2.0).round() as i32
    };
    // Luma: shifted rows, a noisy bright tracking band and fine speckle.
    po[0].par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let shift = shift_for(y);
        let band = band_rows(y);
        let line = &src[0][y * w..(y + 1) * w];
        for (x, o) in row.iter_mut().enumerate() {
            let sx = (x as i32 - shift).clamp(0, w as i32 - 1) as usize;
            let mut v = f32::from(line[sx]);
            if band > 0.0 {
                let n = noise(seed, (y * w + x) as u64 / 3, 7);
                v = v * (1.0 - band * 0.6) + (150.0 + n * 85.0) * band * 0.6;
            }
            let speck = noise(seed, (y * w + x) as u64, 11);
            if speck > 1.0 - 0.004 * intensity {
                v = 225.0;
            }
            *o = v.clamp(16.0, 235.0) as u8;
        }
    });
    // Chroma: shifted with the luma and partly desaturated.
    let cw = w / 2;
    for i in 1..3 {
        po[i].par_chunks_mut(cw).enumerate().for_each(|(y, row)| {
            let shift = shift_for((y * 2).min(h - 1)) / 2 + (intensity * 3.0) as i32;
            let line = &src[i][y * cw..(y + 1) * cw];
            for (x, o) in row.iter_mut().enumerate() {
                let sx = (x as i32 - shift).clamp(0, cw as i32 - 1) as usize;
                let v = f32::from(line[sx]);
                *o = (128.0 + (v - 128.0) * (1.0 - 0.6 * intensity)).round() as u8;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{frame_len, solid};

    const W: u32 = 64;
    const H: u32 = 36;

    fn run(kind: TransitionKind, p: f32) -> Vec<u8> {
        let a = solid(W, H, [50, 100, 150]);
        let b = solid(W, H, [200, 140, 110]);
        let mut out = vec![0u8; frame_len(W, H)];
        blend(kind, &a, &b, &mut out, W, H, p, 42);
        out
    }

    #[test]
    fn every_transition_starts_near_a_and_ends_near_b() {
        for kind in TransitionKind::ALL {
            let start = run(kind, 0.001);
            let end = run(kind, 0.999);
            let mean = |f: &[u8]| {
                f[..(W * H) as usize]
                    .iter()
                    .map(|v| f64::from(*v))
                    .sum::<f64>()
                    / f64::from(W * H)
            };
            assert!(
                (mean(&start) - 50.0).abs() < 8.0,
                "{kind:?} start {}",
                mean(&start)
            );
            assert!(
                (mean(&end) - 200.0).abs() < 8.0,
                "{kind:?} end {}",
                mean(&end)
            );
        }
    }

    #[test]
    fn crossfade_midpoint_is_halfway() {
        let mid = run(TransitionKind::Crossfade, 0.5);
        assert_eq!(mid[0], 125);
        assert_eq!(mid[(W * H) as usize], 120);
    }

    #[test]
    fn fade_black_passes_through_black() {
        let mid = run(TransitionKind::FadeBlack, 0.5);
        assert_eq!(mid[0], 16);
        let mid = run(TransitionKind::FadeWhite, 0.5);
        assert_eq!(mid[0], 235);
    }

    #[test]
    fn blur_keeps_flat_planes_flat() {
        let plane = vec![77u8; 200 * 100];
        assert!(soft_blur(&plane, 200, 100, 12.0).iter().all(|v| *v == 77));
    }
}
