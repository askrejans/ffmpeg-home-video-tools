//! Intro frames: title overlays composited over footage or a colour.

use crate::error::Result;
use crate::events::CancelToken;
use crate::frame::{composite_premultiplied, rgb_to_yuv, solid};
use crate::probe::MediaInfo;
use crate::render::decode::ClipDecoder;
use crate::render::filters::backdrop_graph;
use crate::titles::{TitleBackground, TitleRenderer};
use crate::tools::FfmpegTools;
use rayon::prelude::*;
use std::collections::VecDeque;
use std::sync::Arc;

enum Backdrop {
    Solid([u8; 3]),
    Footage {
        media: Box<MediaInfo>,
        start: f64,
        dim: f32,
        blur: f32,
    },
}

/// Everything needed to produce the intro, prepared once per render.
pub(crate) struct IntroSource {
    renderer: Arc<TitleRenderer>,
    backdrop: Backdrop,
    width: u32,
    height: u32,
    fps: u32,
    tonemap: bool,
    hardware_decode: bool,
}

impl IntroSource {
    /// `first_clip` (media, trim start) provides footage for templates with a
    /// footage background; without it a dark solid colour is used.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        renderer: TitleRenderer,
        background: TitleBackground,
        first_clip: Option<(&MediaInfo, f64)>,
        width: u32,
        height: u32,
        fps: u32,
        tonemap: bool,
        hardware_decode: bool,
    ) -> Self {
        let backdrop = match (background, first_clip) {
            (TitleBackground::Footage { dim, blur }, Some((media, start))) => Backdrop::Footage {
                media: Box::new(media.clone()),
                start,
                dim,
                blur,
            },
            (TitleBackground::Footage { .. }, None) => Backdrop::Solid(rgb_to_yuv(14, 14, 18)),
            (TitleBackground::Solid { r, g, b }, _) => Backdrop::Solid(rgb_to_yuv(r, g, b)),
        };
        Self {
            renderer: Arc::new(renderer),
            backdrop,
            width,
            height,
            fps,
            tonemap,
            hardware_decode,
        }
    }

    pub fn stream(
        &self,
        tools: &FfmpegTools,
        frames: u64,
        cancel: &CancelToken,
    ) -> Result<IntroStream> {
        let backdrop = match &self.backdrop {
            Backdrop::Solid(yuv) => StreamBackdrop::Solid(solid(self.width, self.height, *yuv)),
            Backdrop::Footage {
                media,
                start,
                dim,
                blur,
            } => {
                let video = media.video.as_ref().expect("clips are videos");
                let graph = backdrop_graph(
                    video,
                    self.width,
                    self.height,
                    self.fps,
                    self.tonemap,
                    f64::from(*dim),
                    f64::from(*blur),
                );
                StreamBackdrop::Footage(Box::new(ClipDecoder::spawn(
                    tools,
                    &media.path,
                    *start,
                    frames as f64 / f64::from(self.fps),
                    graph,
                    self.width,
                    self.height,
                    self.hardware_decode,
                    cancel,
                )?))
            }
        };
        let pixels = u64::from(self.width) * u64::from(self.height);
        // Render a few overlays at once in parallel; fewer at large sizes.
        let batch = if pixels > 4_000_000 { 4 } else { 8 }.min(rayon::current_num_threads().max(1));
        Ok(IntroStream {
            renderer: Arc::clone(&self.renderer),
            backdrop,
            index: 0,
            ahead: VecDeque::new(),
            batch,
            width: self.width,
            height: self.height,
        })
    }
}

enum StreamBackdrop {
    Solid(Vec<u8>),
    Footage(Box<ClipDecoder>),
}

pub(crate) struct IntroStream {
    renderer: Arc<TitleRenderer>,
    backdrop: StreamBackdrop,
    index: u64,
    ahead: VecDeque<Vec<u8>>,
    batch: usize,
    width: u32,
    height: u32,
}

impl IntroStream {
    pub fn next_frame(&mut self) -> Result<Vec<u8>> {
        if self.ahead.is_empty() {
            let last = self.renderer.frame_count().saturating_sub(1);
            let len = self.width as usize * self.height as usize * 4;
            let renderer = &self.renderer;
            let rendered: Vec<Vec<u8>> = (self.index..self.index + self.batch as u64)
                .into_par_iter()
                .map(|i| {
                    let mut rgba = vec![0u8; len];
                    renderer.render_frame(i.min(last), &mut rgba);
                    rgba
                })
                .collect();
            self.ahead.extend(rendered);
        }
        let overlay = self.ahead.pop_front().expect("rendered ahead");
        self.index += 1;
        let mut frame = match &mut self.backdrop {
            StreamBackdrop::Solid(f) => f.clone(),
            StreamBackdrop::Footage(decoder) => decoder.next_frame()?,
        };
        let (w, h) = (self.width as usize, self.height as usize);
        composite_premultiplied(&mut frame, w, h, &overlay, w, h, 0, 0);
        Ok(frame)
    }

    pub fn finish(self) {
        if let StreamBackdrop::Footage(decoder) = self.backdrop {
            decoder.finish();
        }
    }
}
