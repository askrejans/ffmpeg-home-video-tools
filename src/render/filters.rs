//! FFmpeg filter graphs used by the renderer.
//!
//! Only LGPL filters are used, so the graphs work with any FFmpeg build.

use crate::probe::{HdrTransfer, VideoInfo};

/// Filters that FFmpeg only builds with `--enable-gpl` (FFmpeg 9.0
/// `configure`). None of them may appear in a graph.
#[cfg(test)]
pub(crate) const GPL_ONLY_FILTERS: &[&str] = &[
    "blackframe",
    "boxblur",
    "boxblur_opencl",
    "colormatrix",
    "cover_rect",
    "cropdetect",
    "delogo",
    "eq",
    "find_rect",
    "fspp",
    "histeq",
    "hqdn3d",
    "interlace",
    "kerndeint",
    "mcdeint",
    "mpdecimate",
    "mptestsrc",
    "nnedi",
    "owdenoise",
    "perspective",
    "phase",
    "pp7",
    "pullup",
    "repeatfields",
    "sab",
    "signature",
    "smartblur",
    "spp",
    "stereo3d",
    "super2xsai",
    "tinterlace",
    "uspp",
    "vaguedenoiser",
];

fn even(value: f64) -> u32 {
    (((value / 2.0).round() as u32) * 2).max(2)
}

/// Largest size with `aspect` that fits inside `w`×`h`.
pub(crate) fn contain(aspect: f64, w: u32, h: u32) -> (u32, u32) {
    if aspect >= f64::from(w) / f64::from(h) {
        (w, even(f64::from(w) / aspect).min(h))
    } else {
        (even(f64::from(h) * aspect).min(w), h)
    }
}

/// Smallest size with `aspect` that covers `w`×`h`.
pub(crate) fn cover(aspect: f64, w: u32, h: u32) -> (u32, u32) {
    if aspect >= f64::from(w) / f64::from(h) {
        (even(f64::from(h) * aspect).max(w), h)
    } else {
        (w, even(f64::from(w) / aspect).max(h))
    }
}

const COLOUR: &str = "in_range=auto:out_range=tv:in_color_matrix=auto:out_color_matrix=bt709";

/// How a clip's shape is fitted to the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fit {
    /// Same shape: scale only.
    Scale,
    /// Nearly the same shape: fill and crop the small excess.
    Crop,
    /// Different shape: fit inside a blurred, dimmed copy of itself.
    BlurredFill,
}

/// Aspect mismatch below which cropping beats a blurred border.
const CROP_TOLERANCE: f64 = 0.08;

pub(crate) fn fit_for(video: &VideoInfo, w: u32, h: u32) -> Fit {
    let mismatch = (video.display_aspect() / (f64::from(w) / f64::from(h)) - 1.0).abs();
    if mismatch < 0.01 {
        Fit::Scale
    } else if mismatch < CROP_TOLERANCE {
        Fit::Crop
    } else {
        Fit::BlurredFill
    }
}

/// Additional orientation applied after FFmpeg's source-metadata autorotation.
pub(crate) fn rotation_filters(rotation: u32) -> Vec<String> {
    match rotation % 360 {
        90 => vec!["transpose=clock".into()],
        180 => vec!["hflip".into(), "vflip".into()],
        270 => vec!["transpose=cclock".into()],
        _ => Vec::new(),
    }
}

/// Deinterlace, tone-map, orient and resample time before canvas scaling.
fn prelude(video: &VideoInfo, fps: u32, tonemap: bool) -> Vec<String> {
    let mut chain = Vec::new();
    if video.interlaced {
        chain.push("bwdif=mode=send_frame:parity=auto:deint=interlaced".to_string());
    }
    if tonemap && video.hdr.is_some() {
        let tin = match video.hdr {
            Some(HdrTransfer::Hlg) => "arib-std-b67",
            _ => "smpte2084",
        };
        chain.push(format!(
            "zscale=tin={tin}:min=bt2020nc:pin=bt2020:t=linear:npl=100,format=gbrpf32le,\
             zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv"
        ));
    }
    chain.extend(rotation_filters(video.manual_rotation));
    chain.push(format!("fps=fps={fps}:start_time=0"));
    chain
}

/// Blurred, dimmed full-canvas background chain (after `prelude`).
fn background_chain(video: &VideoInfo, w: u32, h: u32, dim: f64, blur: f64) -> String {
    let (bw, bh) = (even(f64::from(w) / 8.0), even(f64::from(h) / 8.0));
    let (sw, sh) = cover(video.display_aspect(), bw, bh);
    let sigma = (f64::from(bh) / 18.0 * blur).clamp(1.0, 60.0);
    let keep = (1.0 - dim).clamp(0.0, 1.0);
    format!(
        "scale={sw}:{sh}:flags=bilinear:{COLOUR},crop={bw}:{bh},gblur=sigma={sigma:.2},\
         scale={w}:{h}:flags=bicubic,lutyuv=y=(val-16)*{keep:.3}+16"
    )
}

/// Graph turning stream `[0:IDX]` into canvas-sized `yuv420p` frames `[v]`.
pub(crate) fn clip_graph(video: &VideoInfo, w: u32, h: u32, fps: u32, tonemap: bool) -> String {
    let input = format!("[0:{}]", video.stream_index);
    let prelude = prelude(video, fps, tonemap).join(",");
    let aspect = video.display_aspect();
    match fit_for(video, w, h) {
        Fit::Scale => format!(
            "{input}{prelude},scale={w}:{h}:flags=lanczos:{COLOUR},setsar=1,format=yuv420p[v]"
        ),
        Fit::Crop => {
            let (cw, ch) = cover(aspect, w, h);
            format!(
                "{input}{prelude},scale={cw}:{ch}:flags=lanczos:{COLOUR},crop={w}:{h},setsar=1,format=yuv420p[v]"
            )
        }
        Fit::BlurredFill => {
            let (fw, fh) = contain(aspect, w, h);
            let background = background_chain(video, w, h, 0.18, 1.0);
            format!(
                "{input}{prelude},split=2[bgin][fgin];\
                 [bgin]{background}[bg];\
                 [fgin]scale={fw}:{fh}:flags=lanczos:{COLOUR},setsar=1[fg];\
                 [bg][fg]overlay=x=(W-w)/2:y=(H-h)/2,setsar=1,format=yuv420p[v]"
            )
        }
    }
}

/// Blurred footage behind a title: `dim` 0..1 removes brightness, `blur`
/// scales the blur strength.
pub(crate) fn backdrop_graph(
    video: &VideoInfo,
    w: u32,
    h: u32,
    fps: u32,
    tonemap: bool,
    dim: f64,
    blur: f64,
) -> String {
    format!(
        "[0:{}]{},{},setsar=1,format=yuv420p[v]",
        video.stream_index,
        prelude(video, fps, tonemap).join(","),
        background_chain(video, w, h, dim, blur)
    )
}

/// Audio chain producing 48 kHz stereo float samples starting at the seek point.
pub(crate) const AUDIO_CHAIN: &str =
    "aresample=48000:async=1:first_pts=0,aformat=sample_fmts=flt:channel_layouts=stereo";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::tests::video;

    fn info(w: u32, h: u32, sar: (u32, u32), rotation: u32) -> VideoInfo {
        let mut v = video("x", 1.0, 30.0).video.unwrap();
        v.width = w;
        v.height = h;
        (v.sar_num, v.sar_den) = sar;
        v.rotation = rotation;
        let shown = (f64::from(w) * f64::from(sar.0) / f64::from(sar.1)).round() as u32;
        (v.display_width, v.display_height) = if rotation % 180 == 90 {
            (h, shown)
        } else {
            (shown, h)
        };
        v
    }

    fn filter_names(graph: &str) -> Vec<String> {
        graph
            .split([',', ';'])
            .filter_map(|part| {
                let mut rest = part.trim();
                while rest.starts_with('[') {
                    rest = &rest[rest.find(']')? + 1..];
                }
                let name = rest.split(['=', '[']).next()?.trim();
                (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                    .then(|| name.to_string())
            })
            .collect()
    }

    #[test]
    fn geometry_helpers() {
        assert_eq!(contain(9.0 / 16.0, 1920, 1080), (608, 1080));
        assert_eq!(contain(4.0 / 3.0, 1920, 1080), (1440, 1080));
        assert_eq!(contain(21.0 / 9.0, 1920, 1080), (1920, 822));
        assert_eq!(cover(9.0 / 16.0, 240, 136), (240, 426));
        assert_eq!(cover(16.0 / 9.0, 1080, 1920), (3414, 1920));
    }

    #[test]
    fn picks_fit_by_shape() {
        assert_eq!(
            fit_for(&info(1920, 1080, (1, 1), 0), 1920, 1080),
            Fit::Scale
        );
        assert_eq!(
            fit_for(&info(3840, 2160, (1, 1), 0), 1920, 1080),
            Fit::Scale
        );
        assert_eq!(
            fit_for(&info(1920, 1088, (1, 1), 0), 1920, 1080),
            Fit::Scale
        );
        assert_eq!(fit_for(&info(1920, 1040, (1, 1), 0), 1920, 1080), Fit::Crop);
        assert_eq!(
            fit_for(&info(1440, 1080, (4, 3), 0), 1920, 1080),
            Fit::Scale
        );
        assert_eq!(
            fit_for(&info(1920, 1080, (1, 1), 90), 1920, 1080),
            Fit::BlurredFill
        );
        assert_eq!(
            fit_for(&info(720, 576, (64, 45), 0), 1920, 1080),
            Fit::Scale
        );
        assert_eq!(
            fit_for(&info(640, 480, (1, 1), 0), 1920, 1080),
            Fit::BlurredFill
        );
        assert_eq!(
            fit_for(&info(1920, 1080, (1, 1), 0), 1080, 1920),
            Fit::BlurredFill
        );
    }

    #[test]
    fn portrait_clip_gets_blurred_fill_with_exact_foreground() {
        let g = clip_graph(&info(1920, 1080, (1, 1), 90), 1920, 1080, 30, true);
        assert!(g.starts_with("[0:0]fps=fps=30:start_time=0,split=2"));
        assert!(g.contains("[fgin]scale=608:1080:flags=lanczos"));
        assert!(g.contains("gblur=sigma="));
        assert!(g.contains("overlay=x=(W-w)/2:y=(H-h)/2"));
        assert!(g.ends_with("format=yuv420p[v]"));
    }

    #[test]
    fn interlaced_hdr_chain_order() {
        let mut v = info(1920, 1080, (1, 1), 0);
        v.interlaced = true;
        v.hdr = Some(HdrTransfer::Hlg);
        let g = clip_graph(&v, 1920, 1080, 25, true);
        let bwdif = g.find("bwdif").unwrap();
        let tonemap = g.find("tonemap=").unwrap();
        let fps = g.find("fps=fps=25").unwrap();
        assert!(bwdif < tonemap && tonemap < fps);
        assert!(g.contains("tin=arib-std-b67"));
        let no_tm = clip_graph(&v, 1920, 1080, 25, false);
        assert!(!no_tm.contains("zscale"));
    }

    #[test]
    fn graphs_use_no_gpl_only_filters() {
        let mut cases = Vec::new();
        for (w, h, sar, rot) in [
            (1920, 1080, (1, 1), 0),
            (1920, 1080, (1, 1), 90),
            (1920, 1040, (1, 1), 0),
            (720, 576, (64, 45), 0),
            (640, 480, (1, 1), 180),
        ] {
            let mut v = info(w, h, sar, rot);
            v.interlaced = true;
            v.hdr = Some(HdrTransfer::Pq);
            for (cw, ch) in [(1920, 1080), (1080, 1920), (3840, 2160)] {
                cases.push(clip_graph(&v, cw, ch, 30, true));
                cases.push(backdrop_graph(&v, cw, ch, 30, true, 0.5, 1.0));
            }
        }
        cases.push(AUDIO_CHAIN.to_string());
        for graph in cases {
            for name in filter_names(&graph) {
                assert!(
                    !GPL_ONLY_FILTERS.contains(&name.as_str()),
                    "{name} in {graph}"
                );
            }
        }
    }
}
