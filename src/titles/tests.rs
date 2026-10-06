use super::scene::Kind;
use super::*;

fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[cfg_attr(not(feature = "builtin-templates"), allow(dead_code))]
fn sample_fields() -> BTreeMap<String, String> {
    fields(&[
        ("title", "Summer at the Lake"),
        ("subtitle", "A family holiday in Latvia"),
        ("date", "July 2026"),
    ])
}

fn opaque_pixels(buf: &[u8]) -> usize {
    buf.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count()
}

fn text_layers(r: &TitleRenderer) -> Vec<&scene::TextLayer> {
    r.scene
        .layers
        .iter()
        .filter_map(|l| match &l.kind {
            Kind::Text(t) => Some(&**t),
            _ => None,
        })
        .collect()
}

#[cfg(feature = "builtin-templates")]
#[test]
fn builtins_load_and_render_both_orientations() {
    assert_eq!(
        TitleTemplate::builtin_names(),
        &["clean", "cinematic", "retro"]
    );
    for name in TitleTemplate::builtin_names() {
        let tpl = TitleTemplate::builtin(name).unwrap();
        assert_eq!(tpl.name(), *name);
        assert!((4.0..=7.0).contains(&tpl.duration()));
        assert!(tpl.sound().is_none());
        let names: Vec<String> = tpl.fields().into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["title", "subtitle", "date"]);
        assert!(tpl.fields()[0].required && !tpl.fields()[1].required);
        for (w, h) in [(1920u32, 1080u32), (1080, 1920)] {
            let r = TitleRenderer::new(&tpl, &sample_fields(), w, h, 30).unwrap();
            assert_eq!(r.frame_count(), (tpl.duration() * 30.0).round() as u64);
            let mut buf = vec![0u8; (w * h * 4) as usize];
            let last = r.frame_count() - 1;
            for idx in [0, r.frame_count() / 2, last] {
                r.render_frame(idx, &mut buf);
            }
            r.render_frame(r.frame_count() / 2, &mut buf);
            let visible = opaque_pixels(&buf);
            assert!(
                visible > 1000,
                "{name} {w}x{h}: mid frame nearly empty ({visible})"
            );
            for px in buf.as_chunks::<4>().0 {
                assert!(
                    px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3],
                    "not premultiplied"
                );
            }
        }
    }
}

#[cfg(feature = "builtin-templates")]
#[test]
fn frame_count_follows_duration_and_fps() {
    let tpl = TitleTemplate::builtin("clean").unwrap();
    for fps in [24u32, 25, 30, 60] {
        let r = TitleRenderer::new(&tpl, &sample_fields(), 320, 180, fps).unwrap();
        assert_eq!(
            r.frame_count(),
            (tpl.duration() * fps as f64).round() as u64
        );
    }
}

#[cfg(feature = "builtin-templates")]
#[test]
fn multilingual_text_has_no_missing_glyphs() {
    let samples = [
        "Ziemassvētki Ķemeros — ā č ē ģ ī ķ ļ ņ š ū ž",
        "Лето на даче — Ёлка и Щука",
        "Καλοκαίρι στη Θεσσαλονίκη — ΑΒΓΔ",
    ];
    for name in TitleTemplate::builtin_names() {
        let tpl = TitleTemplate::builtin(name).unwrap();
        for text in samples {
            let f = fields(&[("title", text), ("subtitle", text), ("date", text)]);
            let r = TitleRenderer::new(&tpl, &f, 1280, 720, 25).unwrap();
            assert!(
                !r.scene.uses_system_fonts,
                "{name}: bundled fonts should cover {text:?}"
            );
            let layers = text_layers(&r);
            assert!(layers.len() >= 3);
            for tl in layers {
                assert!(!tl.glyphs.is_empty());
                for g in &tl.glyphs {
                    assert_ne!(g.glyph_id, 0, "{name}: tofu in {:?}", tl.text);
                }
                let drawn = tl
                    .glyphs
                    .iter()
                    .filter(|g| !matches!(tl.shapes[g.shape], text::GlyphShape::Empty))
                    .count();
                let visible = tl.text.chars().filter(|c| !c.is_whitespace()).count();
                assert!(
                    drawn >= visible * 9 / 10,
                    "{name}: {:?} drew {drawn} glyphs",
                    tl.text
                );
            }
            let mut buf = vec![0u8; 1280 * 720 * 4];
            r.render_frame(r.frame_count() / 2, &mut buf);
            assert!(opaque_pixels(&buf) > 1000);
        }
    }
}

#[cfg(feature = "builtin-templates")]
#[test]
fn other_scripts_and_tiny_canvases_do_not_panic() {
    // Scripts the bundled fonts lack fall back to installed fonts when there
    // are any; either way rendering must succeed.
    let f = fields(&[
        ("title", "夏天 🎉 שלום مرحبا"),
        ("subtitle", "ไทย 한국어"),
        ("date", "2026"),
    ]);
    for name in TitleTemplate::builtin_names() {
        let tpl = TitleTemplate::builtin(name).unwrap();
        let r = TitleRenderer::new(&tpl, &f, 640, 360, 25).unwrap();
        let mut buf = vec![0u8; 640 * 360 * 4];
        r.render_frame(r.frame_count() / 2, &mut buf);
        for (w, h) in [(1u32, 1u32), (2, 3), (17, 9), (9, 17)] {
            let r = TitleRenderer::new(&tpl, &sample_fields(), w, h, 30).unwrap();
            let mut buf = vec![0u8; (w * h * 4) as usize];
            for i in 0..r.frame_count() {
                r.render_frame(i, &mut buf);
            }
        }
    }
}

#[cfg(feature = "builtin-templates")]
#[test]
fn required_and_optional_fields() {
    let tpl = TitleTemplate::builtin("clean").unwrap();
    let err = TitleRenderer::new(&tpl, &fields(&[("subtitle", "x")]), 640, 360, 25)
        .err()
        .unwrap();
    assert!(
        matches!(&err, Error::Template(m) if m.contains("\"title\"")),
        "{err}"
    );
    let err = TitleRenderer::new(&tpl, &fields(&[("title", "   ")]), 640, 360, 25)
        .err()
        .unwrap();
    assert!(matches!(err, Error::Template(_)));

    let full = TitleRenderer::new(&tpl, &sample_fields(), 640, 360, 25).unwrap();
    let only_title = TitleRenderer::new(
        &tpl,
        &fields(&[("title", "Hello"), ("subtitle", ""), ("unknown", "ignored")]),
        640,
        360,
        25,
    )
    .unwrap();
    let texts: Vec<&str> = text_layers(&only_title)
        .iter()
        .map(|t| t.text.as_str())
        .collect();
    assert_eq!(texts, ["Hello"]);
    assert_eq!(text_layers(&full).len(), 3);
    // The stack (title + rule) closes up and stays centred on the canvas.
    let top = text_layers(&only_title)[0].pos.top.unwrap();
    let rule_bottom = only_title
        .scene
        .layers
        .iter()
        .find_map(|l| match &l.kind {
            Kind::Shape(s) => Some(s.pos.top.unwrap() + s.h),
            _ => None,
        })
        .unwrap();
    let centre = (top + rule_bottom) / 2.0;
    assert!((centre - 180.0).abs() < 1.0, "stack centre {centre}");
}

#[cfg(feature = "builtin-templates")]
#[test]
fn long_titles_wrap_or_shrink_inside_the_safe_area() {
    let tpl = TitleTemplate::builtin("cinematic").unwrap();
    let long = "The longest and most wonderful summer holiday of all time";
    for (w, h) in [(1920u32, 1080u32), (1080, 1920)] {
        let r = TitleRenderer::new(&tpl, &fields(&[("title", long)]), w, h, 25).unwrap();
        let tl = text_layers(&r)[0];
        assert!(tl.lines.len() <= 2);
        let tr = tl.tracking.values().into_iter().fold(0.0f32, f32::max) * tl.px;
        for l in &tl.lines {
            let lw = l.width + tr * (l.clusters as f32 - 1.0);
            assert!(lw <= w as f32 * 0.85, "{w}x{h}: line {lw}px too wide");
        }
    }
}

#[cfg(feature = "builtin-templates")]
#[test]
fn rendering_is_deterministic() {
    for name in TitleTemplate::builtin_names() {
        let tpl = TitleTemplate::builtin(name).unwrap();
        let r = TitleRenderer::new(&tpl, &sample_fields(), 480, 270, 25).unwrap();
        let r2 = TitleRenderer::new(&tpl, &sample_fields(), 480, 270, 25).unwrap();
        let mut a = vec![0u8; 480 * 270 * 4];
        let mut b = vec![1u8; 480 * 270 * 4];
        for idx in [3, 40, 77] {
            r.render_frame(idx, &mut a);
            r2.render_frame(idx, &mut b);
            assert!(a == b, "{name} frame {idx} differs between renderers");
            r.render_frame(idx, &mut b);
            assert!(a == b, "{name} frame {idx} differs between calls");
        }
    }
}

#[cfg(feature = "builtin-templates")]
#[test]
fn renders_in_parallel() {
    use rayon::prelude::*;
    let tpl = TitleTemplate::builtin("retro").unwrap();
    let r = TitleRenderer::new(&tpl, &sample_fields(), 320, 180, 25).unwrap();
    let sums: Vec<u64> = (0..r.frame_count())
        .into_par_iter()
        .map(|i| {
            let mut buf = vec![0u8; 320 * 180 * 4];
            r.render_frame(i, &mut buf);
            buf.iter().map(|v| *v as u64).sum()
        })
        .collect();
    let mut buf = vec![0u8; 320 * 180 * 4];
    r.render_frame(10, &mut buf);
    assert_eq!(sums[10], buf.iter().map(|v| *v as u64).sum::<u64>());
}

const MINIMAL: &str = r##"{
  "name": "minimal",
  "duration": 2.0,
  "background": {"kind": "solid", "color": "#102030"},
  "sound": "sound.m4a",
  "fields": [
    {"name": "title", "max_chars": 30, "required": true},
    {"name": "note", "max_chars": 30}
  ],
  "layers": [
    {"type": "fill", "gradient": {"kind": "linear", "angle": 90, "stops": [[0, "#00000000"], [1, "#000000AA"]]}},
    {"type": "text", "field": "title", "font": "IBM Plex Sans", "size": 0.1,
     "opacity": [[0, 0], [0.5, 1, "ease_out_back"]], "reveal": [[0, 0], [1, 1]], "reveal_from": "center"},
    {"type": "text", "field": "note", "font": "IBM Plex Sans", "size": 0.04, "y": 0.8},
    {"type": "particles", "kind": "confetti", "count": 20, "colors": ["#ff0000", "#00ff00"]},
    {"type": "particles", "kind": "bokeh", "count": 10},
    {"type": "sweep", "progress": [[0, 0], [2, 1]]},
    {"type": "scanlines", "speed": 0.1},
    {"type": "noise_band", "y": [[0, 0], [2, 1]]}
  ]
}"##;

fn plex_font() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/templates/fonts/IBMPlexSans-Regular.ttf"
    );
    std::fs::read(path).unwrap()
}

#[test]
fn load_custom_template_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("template.json"), MINIMAL).unwrap();
    std::fs::create_dir(dir.path().join("fonts")).unwrap();
    std::fs::write(
        dir.path().join("fonts/IBMPlexSans-Regular.ttf"),
        plex_font(),
    )
    .unwrap();

    // The declared sound must exist.
    let err = TitleTemplate::load(dir.path()).unwrap_err();
    assert!(err.to_string().contains("sound.m4a"), "{err}");
    std::fs::write(dir.path().join("sound.m4a"), b"not really audio").unwrap();

    let tpl = TitleTemplate::load(dir.path()).unwrap();
    assert_eq!(tpl.name(), "minimal");
    assert_eq!(tpl.duration(), 2.0);
    assert_eq!(
        tpl.background(),
        TitleBackground::Solid {
            r: 0x10,
            g: 0x20,
            b: 0x30
        }
    );
    let sound = tpl.sound().unwrap();
    assert!(sound.is_absolute() && sound.ends_with("sound.m4a"));
    assert_eq!(
        tpl.fields(),
        vec![
            FieldSpec {
                name: "title".into(),
                max_chars: 30,
                required: true
            },
            FieldSpec {
                name: "note".into(),
                max_chars: 30,
                required: false
            },
        ]
    );
    let r = TitleRenderer::new(&tpl, &fields(&[("title", "Hi ģ")]), 400, 300, 10).unwrap();
    assert_eq!(r.frame_count(), 20);
    assert!(!r.scene.uses_system_fonts);
    let layers = text_layers(&r);
    assert_eq!(layers.len(), 1, "empty optional field layer is skipped");
    assert_eq!(layers[0].text, "Hi ģ");
    assert!(layers[0].glyphs.iter().all(|g| g.glyph_id != 0));
    let mut buf = vec![0u8; 400 * 300 * 4];
    for i in 0..r.frame_count() {
        r.render_frame(i, &mut buf);
    }
    assert!(opaque_pixels(&buf) > 0);
}

#[test]
fn invalid_templates_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let err = TitleTemplate::load(dir.path()).unwrap_err();
    assert!(matches!(err, Error::Template(_)));
    assert!(err.to_string().contains("template.json"), "{err}");

    std::fs::write(
        dir.path().join("template.json"),
        "{\"name\": \"x\", \"duration\": 3,,}",
    )
    .unwrap();
    let err = TitleTemplate::load(dir.path()).unwrap_err();
    let msg = err.to_string();
    assert!(matches!(err, Error::Template(_)));
    assert!(
        msg.contains("template.json") && msg.contains("line 1"),
        "{msg}"
    );

    let no_fonts = MINIMAL.replace("\"sound\": \"sound.m4a\",", "");
    std::fs::write(dir.path().join("template.json"), no_fonts).unwrap();
    let err = TitleTemplate::load(dir.path()).unwrap_err();
    assert!(err.to_string().contains("IBM Plex Sans"), "{err}");
}

#[cfg(not(feature = "builtin-templates"))]
#[test]
fn builtins_unavailable_without_feature() {
    assert!(TitleTemplate::builtin_names().is_empty());
    assert!(matches!(
        TitleTemplate::builtin("clean"),
        Err(Error::Template(_))
    ));
}

#[cfg(feature = "builtin-templates")]
#[test]
fn unknown_builtin_is_an_error() {
    let err = TitleTemplate::builtin("nope").unwrap_err();
    assert!(err.to_string().contains("clean"), "{err}");
}

#[test]
fn uppercase_follows_greek_convention() {
    assert_eq!(
        scene::display_uppercase("Ελλάδα ΐ"),
        "ΕΛΛΑΔΑ \u{399}\u{308}"
    );
    assert_eq!(scene::display_uppercase("ģimene Ёлка"), "ĢIMENE ЁЛКА");
}

#[test]
fn field_values_are_cleaned_and_capped() {
    assert_eq!(scene::clean_field("  a \n b\t c ", 20), "a b c");
    assert_eq!(scene::clean_field("abcdefghij", 5), "abcd…");
    assert_eq!(scene::clean_field("ābčdēfģ", 7), "ābčdēfģ");
}

/// Writes PNG frames of templates for visual review:
/// `cargo test --release --lib titles::tests::export_preview_frames -- --ignored --nocapture`
///
/// Environment: `TITLE_PREVIEW_DIR` (output directory), `TITLE_PREVIEW_TEMPLATES`
/// (comma-separated built-in names or template directories), `TITLE_PREVIEW_SETS`
/// (`en`, `lv`, `title-only`, `ru`, `world`) and `TITLE_PREVIEW_TIMES` (seconds, e.g. `0.5,2.8`).
#[cfg(feature = "builtin-templates")]
#[test]
#[ignore]
fn export_preview_frames() {
    let out = std::env::var("TITLE_PREVIEW_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("title-previews"));
    std::fs::create_dir_all(&out).unwrap();
    let sets = [
        ("en", sample_fields()),
        (
            "lv",
            fields(&[
                ("title", "Ziemassvētki pie vecvecākiem Ķekavā"),
                ("subtitle", "Mūsu ģimenes svētku vakars"),
                ("date", "24. decembris, 1987"),
            ]),
        ),
        ("title-only", fields(&[("title", "Ελλάδα 2025")])),
        (
            "ru",
            fields(&[
                ("title", "Лето на даче у бабушки"),
                ("subtitle", "Семейный архив"),
                ("date", "Июль 1986"),
            ]),
        ),
        (
            "world",
            fields(&[
                ("title", "夏天 🎉 שלום مرحبا"),
                ("subtitle", "ไทย 한국어"),
                ("date", "2026"),
            ]),
        ),
    ];
    // The "world" set needs installed fonts, so it is only rendered on request.
    let only_sets = std::env::var("TITLE_PREVIEW_SETS")
        .ok()
        .or_else(|| Some("en,lv,title-only,ru".to_string()));
    let sets: Vec<_> = sets
        .into_iter()
        .filter(|(s, _)| {
            only_sets
                .as_ref()
                .is_none_or(|o| o.split(',').any(|x| x == *s))
        })
        .collect();
    let sizes = [(1920u32, 1080u32), (1080, 1920)];
    let names: Vec<String> = match std::env::var("TITLE_PREVIEW_TEMPLATES") {
        Ok(v) => v.split(',').map(str::to_string).collect(),
        Err(_) => TitleTemplate::builtin_names()
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };
    // Optional comma-separated times in seconds, e.g. TITLE_PREVIEW_TIMES=0.5,2.8
    let times: Option<Vec<f64>> = std::env::var("TITLE_PREVIEW_TIMES")
        .ok()
        .map(|v| v.split(',').filter_map(|t| t.trim().parse().ok()).collect());
    for name in &names {
        // Entries containing a path separator are template directories.
        let tpl = if name.contains('/') {
            TitleTemplate::load(Path::new(name)).unwrap()
        } else {
            TitleTemplate::builtin(name).unwrap()
        };
        let name = tpl.name().to_string();
        let backdrop = match tpl.background() {
            TitleBackground::Solid { r, g, b } => tiny_skia::Color::from_rgba8(r, g, b, 255),
            // Mid-grey stand-in for footage so dark effects stay visible.
            TitleBackground::Footage { .. } => tiny_skia::Color::from_rgba8(70, 82, 96, 255),
        };
        for (set, f) in &sets {
            for (w, h) in sizes {
                let r = TitleRenderer::new(&tpl, f, w, h, 25).unwrap();
                let n = r.frame_count();
                let frames: Vec<(String, u64)> = match &times {
                    Some(ts) => ts
                        .iter()
                        .map(|t| (format!("t{t:.2}"), (t * 25.0).round() as u64))
                        .collect(),
                    None => [
                        ("a", n / 10),
                        ("b", n * 3 / 10),
                        ("c", n / 2),
                        ("d", n * 4 / 5),
                    ]
                    .into_iter()
                    .map(|(l, i)| (l.to_string(), i))
                    .collect(),
                };
                for (label, idx) in frames {
                    let mut pm = tiny_skia::Pixmap::new(w, h).unwrap();
                    pm.fill(backdrop);
                    let mut buf = vec![0u8; (w * h * 4) as usize];
                    r.render_frame(idx, &mut buf);
                    let overlay = tiny_skia::PixmapRef::from_bytes(&buf, w, h).unwrap();
                    pm.draw_pixmap(
                        0,
                        0,
                        overlay,
                        &tiny_skia::PixmapPaint::default(),
                        tiny_skia::Transform::identity(),
                        None,
                    );
                    let file = out.join(format!("{name}-{set}-{w}x{h}-{label}.png"));
                    pm.save_png(&file).unwrap();
                }
            }
        }
    }
    println!("preview frames written to {}", out.display());
}

/// Timing check: `cargo test --release --lib titles::tests::bench_frame_times -- --ignored --nocapture`
#[cfg(feature = "builtin-templates")]
#[test]
#[ignore]
fn bench_frame_times() {
    for name in TitleTemplate::builtin_names() {
        let tpl = TitleTemplate::builtin(name).unwrap();
        for (w, h) in [(1920u32, 1080u32), (3840, 2160)] {
            let t0 = std::time::Instant::now();
            let r = TitleRenderer::new(&tpl, &sample_fields(), w, h, 30).unwrap();
            let setup = t0.elapsed();
            let mut buf = vec![0u8; (w * h * 4) as usize];
            let n = r.frame_count();
            let t1 = std::time::Instant::now();
            let mut worst = std::time::Duration::ZERO;
            let mut count = 0;
            for idx in (0..n).step_by(5) {
                let t = std::time::Instant::now();
                r.render_frame(idx, &mut buf);
                worst = worst.max(t.elapsed());
                count += 1;
            }
            let avg = t1.elapsed() / count;
            println!(
                "{name:>10} {w}x{h}: setup {:>7.1} ms, frame avg {:>6.1} ms, worst {:>6.1} ms",
                setup.as_secs_f64() * 1e3,
                avg.as_secs_f64() * 1e3,
                worst.as_secs_f64() * 1e3
            );
        }
    }
}
