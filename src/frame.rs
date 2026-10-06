//! Raw `yuv420p` frames (BT.709, limited range) and compositing helpers.

use rayon::prelude::*;

/// Bytes in one `yuv420p` frame.
pub(crate) fn frame_len(width: u32, height: u32) -> usize {
    let (w, h) = (width as usize, height as usize);
    w * h + 2 * (w / 2) * (h / 2)
}

pub(crate) fn planes_mut(buf: &mut [u8], w: usize, h: usize) -> (&mut [u8], &mut [u8], &mut [u8]) {
    let (y, rest) = buf.split_at_mut(w * h);
    let (u, v) = rest.split_at_mut((w / 2) * (h / 2));
    (y, u, v)
}

pub(crate) fn planes(buf: &[u8], w: usize, h: usize) -> (&[u8], &[u8], &[u8]) {
    let (y, rest) = buf.split_at(w * h);
    let (u, v) = rest.split_at((w / 2) * (h / 2));
    (y, u, v)
}

const KR: f32 = 0.2126;
const KB: f32 = 0.0722;
const KG: f32 = 1.0 - KR - KB;

/// BT.709 limited-range Y, Cb, Cr contributions of a (premultiplied) colour
/// with components in 0..=1, *excluding* the 16/128 offsets.
#[inline]
fn ycc(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let y = KR * r + KG * g + KB * b;
    let cb = (b - y) / (2.0 * (1.0 - KB));
    let cr = (r - y) / (2.0 * (1.0 - KR));
    (219.0 * y, 224.0 * cb, 224.0 * cr)
}

/// sRGB colour to limited-range BT.709 YUV.
pub(crate) fn rgb_to_yuv(r: u8, g: u8, b: u8) -> [u8; 3] {
    let (y, u, v) = ycc(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    [
        (16.0 + y).round().clamp(0.0, 255.0) as u8,
        (128.0 + u).round().clamp(0.0, 255.0) as u8,
        (128.0 + v).round().clamp(0.0, 255.0) as u8,
    ]
}

/// A solid frame.
pub(crate) fn solid(width: u32, height: u32, yuv: [u8; 3]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut buf = vec![0u8; frame_len(width, height)];
    let (y, u, v) = planes_mut(&mut buf, w, h);
    y.fill(yuv[0]);
    u.fill(yuv[1]);
    v.fill(yuv[2]);
    buf
}

pub(crate) const BLACK: [u8; 3] = [16, 128, 128];
pub(crate) const WHITE: [u8; 3] = [235, 128, 128];

/// Alpha-composite a premultiplied RGBA image (`rw`×`rh`, both even) onto a
/// frame at the even offset (`x0`, `y0`). Pixels outside the frame are
/// ignored.
#[allow(clippy::too_many_arguments)]
pub(crate) fn composite_premultiplied(
    frame: &mut [u8],
    fw: usize,
    fh: usize,
    rgba: &[u8],
    rw: usize,
    rh: usize,
    x0: usize,
    y0: usize,
) {
    debug_assert!(
        x0.is_multiple_of(2)
            && y0.is_multiple_of(2)
            && rw.is_multiple_of(2)
            && rh.is_multiple_of(2)
    );
    debug_assert_eq!(rgba.len(), rw * rh * 4);
    let cw = fw / 2;
    let (yp, up, vp) = planes_mut(frame, fw, fh);
    let first = y0 / 2;
    let last = ((y0 + rh) / 2).min(fh / 2);
    if first >= last {
        return;
    }
    let x_end = (x0 + rw).min(fw);
    yp.par_chunks_mut(fw * 2)
        .zip(up.par_chunks_mut(cw).zip(vp.par_chunks_mut(cw)))
        .enumerate()
        .skip(first)
        .take(last - first)
        .for_each(|(cy, (yrows, (urow, vrow)))| {
            let ry = cy * 2 - y0;
            for cx in (x0 / 2)..(x_end / 2) {
                let rx = cx * 2 - x0;
                let mut sum = [0f32; 4];
                let mut any = false;
                for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    let o = ((ry + dy) * rw + rx + dx) * 4;
                    let a = f32::from(rgba[o + 3]) / 255.0;
                    if a == 0.0 {
                        continue;
                    }
                    any = true;
                    let (r, g, b) = (
                        f32::from(rgba[o]) / 255.0,
                        f32::from(rgba[o + 1]) / 255.0,
                        f32::from(rgba[o + 2]) / 255.0,
                    );
                    let (yc, _, _) = ycc(r, g, b);
                    let yi = dy * fw + cx * 2 + dx;
                    let bg = f32::from(yrows[yi]);
                    yrows[yi] = (bg * (1.0 - a) + 16.0 * a + yc).round().clamp(0.0, 255.0) as u8;
                    sum[0] += r;
                    sum[1] += g;
                    sum[2] += b;
                    sum[3] += a;
                }
                if any {
                    let (r, g, b, a) = (sum[0] / 4.0, sum[1] / 4.0, sum[2] / 4.0, sum[3] / 4.0);
                    let (_, cb, cr) = ycc(r, g, b);
                    let ub = f32::from(urow[cx]);
                    let vb = f32::from(vrow[cx]);
                    urow[cx] = (ub * (1.0 - a) + 128.0 * a + cb).round().clamp(0.0, 255.0) as u8;
                    vrow[cx] = (vb * (1.0 - a) + 128.0 * a + cr).round().clamp(0.0, 255.0) as u8;
                }
            }
        });
}

/// A watermark image, scaled and positioned once for a canvas.
pub(crate) struct Stamp {
    rgba: Vec<u8>,
    w: usize,
    h: usize,
    x: usize,
    y: usize,
}

impl Stamp {
    /// Prepare `png` for a `fw`×`fh` canvas.
    pub(crate) fn new(
        png: &std::path::Path,
        fw: u32,
        fh: u32,
        wm: &crate::project::Watermark,
    ) -> crate::Result<Self> {
        use crate::project::Anchor;
        use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};
        let source = Pixmap::load_png(png).map_err(|e| {
            crate::Error::InvalidProject(format!("watermark {}: {e}", png.display()))
        })?;
        let target_w = ((f64::from(fw) * wm.width_fraction).round() as u32 / 2 * 2).max(2);
        let scale = target_w as f32 / source.width() as f32;
        let target_h = ((source.height() as f32 * scale).round() as u32 / 2 * 2).max(2);
        let mut scaled = Pixmap::new(target_w, target_h).expect("non-empty watermark");
        scaled.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &PixmapPaint {
                opacity: wm.opacity as f32,
                quality: FilterQuality::Bicubic,
                ..Default::default()
            },
            Transform::from_scale(scale, target_h as f32 / source.height() as f32),
            None,
        );
        let margin = (f64::from(fw.min(fh)) * wm.margin_fraction).round() as u32;
        let right = fw.saturating_sub(target_w + margin);
        let bottom = fh.saturating_sub(target_h + margin);
        let (x, y) = match wm.anchor {
            Anchor::TopLeft => (margin, margin),
            Anchor::TopRight => (right, margin),
            Anchor::BottomLeft => (margin, bottom),
            Anchor::BottomRight => (right, bottom),
        };
        Ok(Self {
            rgba: scaled.take(),
            w: target_w as usize,
            h: target_h as usize,
            x: (x / 2 * 2) as usize,
            y: (y / 2 * 2) as usize,
        })
    }

    pub(crate) fn apply(&self, frame: &mut [u8], fw: usize, fh: usize) {
        composite_premultiplied(frame, fw, fh, &self.rgba, self.w, self.h, self.x, self.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bt709_reference_colours() {
        assert_eq!(rgb_to_yuv(0, 0, 0), BLACK);
        assert_eq!(rgb_to_yuv(255, 255, 255), WHITE);
        // BT.709 limited-range red: Y≈63, Cb≈102, Cr=240.
        assert_eq!(rgb_to_yuv(255, 0, 0), [63, 102, 240]);
    }

    #[test]
    fn composites_opaque_and_transparent_pixels() {
        let (w, h) = (4usize, 4usize);
        let mut frame = solid(w as u32, h as u32, BLACK);
        // 2×2 opaque white in the top-left block, transparent elsewhere.
        let mut rgba = vec![0u8; 4 * 4 * 4];
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            rgba[(y * 4 + x) * 4..][..4].copy_from_slice(&[255, 255, 255, 255]);
        }
        composite_premultiplied(&mut frame, w, h, &rgba, 4, 4, 0, 0);
        let (y, u, v) = planes(&frame, w, h);
        assert_eq!(&y[..2], &[235, 235]);
        assert_eq!(y[2], 16);
        assert_eq!((u[0], v[0]), (128, 128));
        assert_eq!(y[w * 2], 16);
    }

    #[test]
    fn half_transparent_blends() {
        let mut frame = solid(2, 2, BLACK);
        let rgba = [128u8, 128, 128, 128].repeat(4); // 50 % white, premultiplied
        composite_premultiplied(&mut frame, 2, 2, &rgba, 2, 2, 0, 0);
        assert!((frame[0] as i32 - 126).abs() <= 1, "got {}", frame[0]);
    }
}
