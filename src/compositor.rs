//! Video transition compositing for hosts with decoded packed-pixel frames.
//!
//! Packed frames are converted through the same limited-range BT.709 YUV
//! compositor used by the renderer. They must be opaque RGBA or BGRA.

use crate::{Error, Result, project::TransitionKind};
use rayon::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba,
    Bgra,
}

/// Blend two equally sized, tightly packed opaque frames. Dimensions must be
/// even; output is opaque and uses the same channel order as the inputs.
#[allow(clippy::too_many_arguments)]
pub fn blend(
    kind: TransitionKind,
    a: &[u8],
    b: &[u8],
    out: &mut [u8],
    width: u32,
    height: u32,
    progress: f32,
    seed: u64,
    format: PixelFormat,
) -> Result<()> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    if width < 2
        || height < 2
        || !width.is_multiple_of(2)
        || !height.is_multiple_of(2)
        || width > 8192
        || height > 8192
        || expected != Some(a.len())
        || a.len() != b.len()
        || a.len() != out.len()
        || !progress.is_finite()
    {
        return Err(Error::InvalidProject(
            "transition frames must have matching even dimensions and packed RGBA/BGRA buffers"
                .into(),
        ));
    }
    if progress <= 0.0 {
        out.copy_from_slice(a);
        return Ok(());
    }
    if progress >= 1.0 {
        out.copy_from_slice(b);
        return Ok(());
    }
    let a = to_yuv(a, width, height, format);
    let b = to_yuv(b, width, height, format);
    let mut yuv = vec![0; crate::frame::frame_len(width, height)];
    crate::transitions::blend(kind, &a, &b, &mut yuv, width, height, progress, seed);
    from_yuv(&yuv, out, width, height, format);
    Ok(())
}

fn to_yuv(input: &[u8], width: u32, height: u32, format: PixelFormat) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut output = vec![0; crate::frame::frame_len(width, height)];
    let (yp, up, vp) = crate::frame::planes_mut(&mut output, w, h);
    yp.par_chunks_mut(w * 2)
        .zip(up.par_chunks_mut(w / 2).zip(vp.par_chunks_mut(w / 2)))
        .enumerate()
        .for_each(|(cy, (yr, (ur, vr)))| {
            for cx in 0..w / 2 {
                let (mut u, mut v) = (0u32, 0u32);
                for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    let offset = ((cy * 2 + dy) * w + cx * 2 + dx) * 4;
                    let p = &input[offset..offset + 4];
                    let (r, g, b) = match format {
                        PixelFormat::Rgba => (p[0], p[1], p[2]),
                        PixelFormat::Bgra => (p[2], p[1], p[0]),
                    };
                    let colour = crate::frame::rgb_to_yuv(r, g, b);
                    yr[dy * w + cx * 2 + dx] = colour[0];
                    u += u32::from(colour[1]);
                    v += u32::from(colour[2]);
                }
                ur[cx] = ((u + 2) / 4) as u8;
                vr[cx] = ((v + 2) / 4) as u8;
            }
        });
    output
}
pub(crate) fn from_yuv(
    input: &[u8],
    output: &mut [u8],
    width: u32,
    height: u32,
    format: PixelFormat,
) {
    let (w, h) = (width as usize, height as usize);
    let (yp, up, vp) = crate::frame::planes(input, w, h);
    output
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, p) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let yy = (f32::from(yp[y * w + x]) - 16.0) / 219.0;
                let cb = (f32::from(up[(y / 2) * (w / 2) + x / 2]) - 128.0) / 224.0;
                let cr = (f32::from(vp[(y / 2) * (w / 2) + x / 2]) - 128.0) / 224.0;
                let (r, b) = (yy + 1.5748 * cr, yy + 1.8556 * cb);
                let g = (yy - 0.2126 * r - 0.0722 * b) / 0.7152;
                let quant = |c: f32| (c * 255.0).round().clamp(0.0, 255.0) as u8;
                let (r, g, b) = (quant(r), quant(g), quant(b));
                match format {
                    PixelFormat::Rgba => p.copy_from_slice(&[r, g, b, 255]),
                    PixelFormat::Bgra => p.copy_from_slice(&[b, g, r, 255]),
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_and_channel_orders() {
        for format in [PixelFormat::Rgba, PixelFormat::Bgra] {
            let a = [255, 0, 0, 255].repeat(16);
            let b = [0, 0, 255, 255].repeat(16);
            let mut output = vec![0; 64];
            blend(
                TransitionKind::Crossfade,
                &a,
                &b,
                &mut output,
                4,
                4,
                0.0,
                0,
                format,
            )
            .unwrap();
            assert_eq!(output, a);
            blend(
                TransitionKind::Crossfade,
                &a,
                &b,
                &mut output,
                4,
                4,
                1.0,
                0,
                format,
            )
            .unwrap();
            assert_eq!(output, b);
            blend(
                TransitionKind::FadeBlack,
                &a,
                &b,
                &mut output,
                4,
                4,
                0.5,
                0,
                format,
            )
            .unwrap();
            assert!(
                output
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| *p == [0, 0, 0, 255])
            );
        }
    }
    #[test]
    fn rejects_wrong_buffers() {
        assert!(
            blend(
                TransitionKind::Wipe,
                &[0; 64],
                &[0; 64],
                &mut [0; 64],
                3,
                4,
                0.5,
                0,
                PixelFormat::Rgba
            )
            .is_err()
        );
    }
}
