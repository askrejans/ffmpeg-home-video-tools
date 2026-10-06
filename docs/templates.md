# Title templates

An intro title is an animated overlay — title, subtitle, date and decorative
effects — drawn over the start of the movie. Each look is a **template**: a
directory holding a `template.json` file that describes the layout and the
animation, plus the fonts it uses. The same template adapts to any canvas size
and to both landscape (16:9) and portrait (9:16) video.

```
my-template/
├── template.json      # layout, fields, layers and keyframes (this document)
├── fonts/             # every .ttf / .otf / .ttc / .otc here is loaded
│   ├── MyFont-Regular.ttf
│   └── MyFont-Bold.ttf
└── sound.m4a          # optional, declared with "sound"
```

The library ships three templates — `clean`, `cinematic` and `retro` — in
`templates/` (with the shared fonts in `templates/fonts/`). They are embedded
in the library when the `builtin-templates` feature is enabled (it is by
default) and are good starting points for your own: copy a directory, copy the
fonts you need into its `fonts/` folder (or point `"fonts"` at them) and edit.

## Using a template from Rust

```rust
use std::collections::BTreeMap;
use std::path::Path;
use ffmpeg_video_processor::titles::{TitleRenderer, TitleTemplate};

fn render_intro() -> ffmpeg_video_processor::Result<()> {
    let template = TitleTemplate::load(Path::new("my-template"))?; // or TitleTemplate::builtin("clean")?
    let mut fields = BTreeMap::new();
    fields.insert("title".to_string(), "Summer at the Lake".to_string());
    fields.insert("date".to_string(), "July 2026".to_string());

    let renderer = TitleRenderer::new(&template, &fields, 1920, 1080, 25)?;
    let mut frame = vec![0u8; 1920 * 1080 * 4];
    for i in 0..renderer.frame_count() {
        renderer.render_frame(i, &mut frame); // premultiplied RGBA8
        // ... composite `frame` over the background ...
    }
    Ok(())
}
```

* `TitleRenderer::new` does all the expensive work — font loading, shaping,
  line breaking, glyph outlines, noise tiles, vignette maps. `render_frame`
  only draws, so a renderer is `Send + Sync` and frames can be rendered from
  many threads at once.
* Frames are **premultiplied** sRGB RGBA8, row-major. Transparent pixels show
  the background chosen by the template (`background()`): blurred, dimmed
  footage of the first clip or a solid colour.
* `frame_count()` is `round(duration × fps)` (at least 1); frame `i` shows
  time `i / fps` seconds. Indices past the end render the last frame.
* The render pipeline cross-fades from the intro into the first clip over
  roughly the last second, so templates should hold or fade their content
  during that time.

## Units and layout

| Quantity | Unit |
|----------|------|
| Times, durations | seconds from the start of the intro |
| `x`, `y` positions | fraction of the frame's width / height (0 = left/top, 1 = right/bottom) |
| Sizes, offsets, gaps, blur radii (`size`, `dx`, `dy`, `gap`, `width`, `height`, `blur`, `radius`, …) | fraction of **s**, the canvas's *short side* — the height of a landscape canvas, the width of a portrait one |
| `max_width` | fraction of the canvas width |
| Letterbox `size`, noise band `height`/`y` | fraction of the canvas height |
| `tracking` | em (fraction of the font size) |
| `rotation`, `skew`, `angle` | degrees (rotation is clockwise) |
| Colours | `"#rgb"`, `"#rgba"`, `"#rrggbb"` or `"#rrggbbaa"` (sRGB, straight alpha) |

Because sizes follow the short side, text keeps the same physical size on a
1920×1080 and a 1080×1920 canvas, and 4K output is simply a scaled-up 1080p.

**Portrait or landscape** is chosen per render: when the canvas is taller than
it is wide (`height > width`) the template's portrait overrides are applied;
square and wider canvases use the landscape layout.

**Frames.** Every positioned layer has a `"frame"`: `"canvas"` (default) or
`"safe"`. In the safe frame, `x`/`y` are fractions of the *safe area* — the
canvas minus the `safe_area` margins — which keeps corner decorations clear of
the edges and of phone UI. Text is always fitted inside the safe area
horizontally.

## Top-level keys

| Key | Required | Description |
|-----|----------|-------------|
| `format` | no | Format version; this library reads `1`. Newer numbers are rejected with a clear error. |
| `name` | yes | Template name returned by `name()`. |
| `description` | no | Free text for humans. |
| `duration` | yes | Length in seconds (0 < duration ≤ 60, typically 4–7). |
| `background` | yes | What shows through transparent pixels, see below. |
| `fields` | yes | User-fillable text fields, see below. |
| `layers` | yes | Layers drawn in order, first at the bottom. |
| `sound` | no | Audio file path relative to the template directory. |
| `fonts` | no | Extra font files or directories relative to the template directory, loaded in addition to `fonts/`. |
| `fallback_fonts` | no | Font families tried, in order, for characters a layer's own fonts lack. |
| `safe_area` | no | `{"x": 0.06, "y": 0.08}` — margins as fractions of width / height (each below 0.45). |
| `stacks` | no | Named vertical stacks of layers, see [Stacks](#stacks). |
| `portrait` | no | Overrides for `safe_area` and `stacks` on portrait canvases. |

Unknown keys are errors everywhere in the file, so typos are reported instead
of silently ignored.

### Background

```json
"background": { "kind": "footage", "dim": 0.45, "blur": 1.0 }
"background": { "kind": "solid", "color": "#101014" }
```

`footage` asks the pipeline for blurred footage of the first clip; `dim`
(0..1, default 0.45) is the fraction of brightness removed and `blur` (0..10,
default 1) scales the blur strength. `solid` is a flat colour (alpha is
ignored). A template can still paint its own backdrop on top with a `fill`
layer.

### Fields

```json
"fields": [
  { "name": "title",    "label": "Title",    "max_chars": 60, "required": true },
  { "name": "subtitle", "label": "Subtitle", "max_chars": 80 },
  { "name": "date",     "label": "Date",     "max_chars": 40 }
]
```

`name` is the key in the field map passed to `TitleRenderer::new`; `label` is
an optional human-readable name. Values are cleaned before use: runs of white
space (including line breaks) collapse to one space, control characters are
removed, and values longer than `max_chars` characters are shortened with a
trailing `…`. A `required` field that is missing or blank makes
`TitleRenderer::new` fail with `Error::Template`. A layer bound to an empty
optional field is not drawn at all, and stacks close the gap it leaves. Keys
in the map that the template does not declare are ignored.

### Fonts

All `.ttf`, `.otf`, `.ttc` and `.otc` files in `fonts/` are loaded, plus the
files and directories listed in `"fonts"`. Layers refer to fonts by **family
name** (`"font": "IBM Plex Sans"`) and pick the closest available `weight`
(100–900) and `italic` style. Every family named by a layer or in
`fallback_fonts` must be among the template's fonts; otherwise loading fails
and the error lists the families that are available.

Static (single-weight) font files are the simplest choice. Variable fonts
with a weight (`wght`) axis also work: the requested weight is used directly,
within the axis range; other axes keep their defaults. Make sure the licence
of every font you ship allows redistribution (the bundled fonts use the SIL
Open Font License; see `templates/fonts/README.md`).

**Fallback.** Text is split into runs by font coverage. For every character the
renderer tries, in order: the layer's font, the layer's `"fallback"` families,
the template's `fallback_fonts`, then every other font of the template.
Characters that no template font covers (for example CJK or emoji with the
bundled fonts) are drawn with fonts installed on the system, when there are
any. Runs set in a fallback font are scaled so their cap height matches the
layer's font. Bundled fonts always win over installed fonts with the same
family name, so output is identical on every machine as long as the template's
fonts cover the text.

Shaping is done with full Unicode support (ligatures, kerning, combining
marks, right-to-left scripts). With `"uppercase": true`, Greek capitals follow
the typographic convention of dropping the tonos accent.

### Sound

`"sound": "sound.m4a"` names an audio file inside the template directory. The
file must exist when the template is loaded; `TitleTemplate::sound()` returns
its absolute path, and the pipeline plays it under the intro. Built-in
templates have no sound.

## Portrait overrides

Any layer may carry a `"portrait"` object; on portrait canvases it is merged
into the layer before the layer is read. Objects merge key by key (so
`"portrait": {"glow": {"radius": 0.03}}` keeps the other glow settings),
everything else is replaced, and `null` clears an optional setting:

```json
{
  "type": "letterbox", "aspect": 2.39,
  "portrait": { "aspect": null, "size": 0.075 }
}
```

The top-level `"portrait"` object may override `safe_area` and `stacks` in the
same way. Both layouts are validated when the template loads.

## Stacks

Titles have a variable number of lines and optional subtitles, so fixed `y`
positions rarely work. A **stack** lays its members out vertically, in layer
order, and positions the whole group:

```json
"stacks": { "main": { "y": 0.5, "valign": "middle" } },
"layers": [
  { "type": "text", "field": "title",    "stack": "main", ... },
  { "type": "rect",                      "stack": "main", "gap": 0.04, ... },
  { "type": "text", "field": "subtitle", "stack": "main", "gap": 0.045, ... }
]
```

Stack keys: `y` (fraction of the frame height, default 0.5), `valign`
(`top`, `middle` (default) or `bottom`: which edge of the group sits at `y`)
and `frame` (`canvas` or `safe`). A member's `gap` is the space above it
(ignored for the first member present). Members that are not drawn (empty
field, unmet `requires`, `"enabled": false`) take no space. A text block's
height runs from the cap height of its first line to the baseline of its last
line, so gaps are measured baseline-to-cap-top as typographers do. Stack
members keep their own `x`, `align`, `dx` and `dy` (use `dy` to animate a
member moving vertically); their `y` and `valign` are ignored.

## Animation

Every key marked *animatable* below accepts a constant or keyframes:

```json
"opacity": 0.8
"opacity": [[0.2, 0], [1.0, 1, "ease_out"], [3.8, 1], [4.6, 0]]
"opacity": { "keys": [[0.2, 0], [1.0, 1]], "ease": "ease_in_out" }
"color":   [[0, "#FFFFFF"], [2, "#FFD9A0"]]
```

A keyframe is `[time, value]` or `[time, value, easing]`; times are seconds
from the start of the intro and must not decrease. Before the first key the
first value holds, after the last key the last value holds. The easing of a
keyframe shapes the segment that **ends** at it. A key without an easing uses
the `"ease"` of its object form, else the layer's `"ease"` key, else `linear`.
Two keys at the same time make an instant jump.

Easings: `linear`, `step` (hold, then jump at the key), `ease_in`,
`ease_out`, `ease_in_out` (cubic), `ease_in_quad`, `ease_out_quad`,
`ease_in_out_quad`, `ease_in_expo`, `ease_out_expo`, `ease_in_back`,
`ease_out_back` (overshoots), `ease_in_out_sine`, and CSS-style cubic Béziers
written as `"cubic-bezier(0.2, 0.8, 0.2, 1)"` or `[0.2, 0.8, 0.2, 1]`.

Colours interpolate in sRGB.

## Layers

Layers are drawn in order, each over the previous ones, into a transparent
overlay. Keys every layer accepts:

| Key | Default | Description |
|-----|---------|-------------|
| `type` | required | One of the layer types below. |
| `id` | — | Name used by `width_of`. Must be unique. |
| `enabled` | `true` | `false` removes the layer (handy in `portrait` overrides). |
| `requires` | `[]` | Field names that must all be non-empty for the layer to be drawn. |
| `start`, `end` | whole intro | The layer is only drawn between these times (seconds). |
| `opacity` | `1` | Animatable, 0..1. Multiplies everything the layer draws. |
| `ease` | `linear` | Default easing for this layer's keyframes. |
| `portrait` | — | Overrides merged in on portrait canvases. |

Text and shape layers also take **placement** keys:

| Key | Default | Description |
|-----|---------|-------------|
| `x`, `y` | `0.5` | Animatable anchor position, fractions of the frame. |
| `frame` | `canvas` | `canvas` or `safe`. |
| `align` | `center` | `left`, `center` or `right`: which side of the layer sits at `x`. |
| `valign` | `middle` | `top`, `middle`, `bottom` or `baseline` (text: `y` is the first baseline). |
| `stack`, `gap` | — | Stack membership and the space above this member (s units). |
| `dx`, `dy` | `0` | Animatable offsets in s units, added to the position. |
| `scale` | `1` | Animatable scale around the layer's centre. |
| `rotation` | `0` | Animatable rotation in degrees around the layer's centre. |

### `text`

| Key | Default | Description |
|-----|---------|-------------|
| `field` / `text` | one required | Bind to a field, or show a literal string. |
| `font` | required | Font family name. |
| `weight` | `400` | 1–1000; the nearest available weight is used. |
| `italic` | `false` | Use the family's italic face (synthesised if there is none). |
| `fallback` | `[]` | Families to try before the template's `fallback_fonts`. |
| `size` | `0.05` | Font size in s units. |
| `color` | `#FFFFFF` | Animatable colour. |
| `max_width` | `1` | Maximum line width, fraction of canvas width (also limited by the safe area). |
| `max_lines` | `1` | 1–3. Longer text wraps at spaces into balanced lines before it shrinks. |
| `line_height` | `1.15` | Baseline distance as a multiple of the font size. |
| `uppercase` | `false` | Show the text in capitals. |
| `tracking` | `0` | Animatable letter spacing in em. |
| `skew` | `0` | Slant in degrees (positive leans right). |
| `blur` | `0` | Animatable blur radius in s units (e.g. a focus pull). |
| `reveal` | `1` | Animatable 0..1 wipe that uncovers the text. |
| `reveal_from` | `left` | `left`, `right`, `top`, `bottom` or `center`. |
| `reveal_softness` | `0.15` | Width of the wipe's soft edge, fraction of the text block. |
| `stagger` | — | Per-glyph delay, see below. |
| `glyph` | — | Per-glyph animation, see below. |
| `shadow` | — | `{ "color": "#000000", "opacity": 0.5, "blur": 0.008, "dx": 0, "dy": 0.003 }`; `opacity` is animatable. |
| `glow` | — | `{ "color": "#FFFFFF", "opacity": 0.35, "radius": 0.02 }`; `opacity` is animatable. |
| `chroma` | — | VHS-style colour fringes: red and cyan copies offset sideways. `{ "offset": 0.003, "opacity": 0.6, "left": "#FF1F4D", "right": "#1FD1FF" }`; `offset` and `opacity` are animatable. |
| `jitter` | — | Horizontal tracking-error jumps: `{ "amount": 0.004, "rate": 12, "seed": 1 }`; `amount` (s units) is animatable — key it down to 0 so the picture "locks". |
| `sweep` | — | A light passing over the letters: `{ "progress": [[2, 0], [3.4, 1]], "angle": 20, "width": 0.12, "color": "#FFFFFF", "opacity": 0.9 }`. `progress` 0→1 moves the band from before the text to past it; the letters brighten and bloom under it. |

**Fitting.** The text is first set on one line at `size`. If it is too wide
for the available width (taking the largest `tracking` of the animation into
account) it is shrunk by up to 15 %; beyond that it wraps at spaces into up to
`max_lines` balanced lines, and if it still does not fit, the wrapped text is
shrunk until it does.

**Stagger and per-glyph animation.** `"stagger": 0.03` delays each grapheme
(letter with its accents) by 0.03 s more than the previous one; the long form
is `{ "each": 0.03, "order": "forward", "seed": 0 }` with `order` one of
`forward`, `reverse`, `center` (from the middle out), `edges` (from both ends
in) or `random` (seeded shuffle). The `glyph` object animates every grapheme
on its own delayed clock — its keyframe times are those of the first grapheme:

```json
"stagger": { "each": 0.025 },
"glyph": {
  "opacity":  [[0.2, 0], [0.8, 1, "ease_out"]],
  "dy":       [[0.2, 0.04], [1.1, 0, "ease_out"]],
  "scale":    [[0.2, 1.4], [0.8, 1, "ease_out_back"]],
  "rotation": [[0.2, -20], [0.8, 0]]
}
```

`glyph` accepts `opacity`, `dx`, `dy` (s units), `scale` and `rotation`; scale
and rotation pivot around the middle of each letter.

### `rect`, `line`, `ellipse`, `triangle`

Simple shapes, e.g. an underline that draws out from the centre. `line` is a
synonym of `rect`; triangles point right (rotate them for other directions).

| Key | Default | Description |
|-----|---------|-------------|
| `width` | `0.1` | Width in s units. |
| `width_of` | — | Id of a text layer: the shape takes that text's widest line width (at the end of its tracking animation) × `width_factor`, plus `width` if given. If that text is not drawn, neither is the shape. |
| `width_factor` | `1` | See `width_of`. |
| `height` | `0.004` (rect), else the width | Height in s units. |
| `radius` | `0` | Corner radius in s units (rect). |
| `color` | `#FFFFFF` | Animatable colour. |
| `reveal` | `1` | Animatable 0..1. Rects grow from `reveal_from`; ellipses and triangles scale up. |
| `reveal_from` | `left` | `left`, `right`, `center`, `top` or `bottom`. |

### `letterbox`

Black bars at the top and bottom that slide in.

| Key | Default | Description |
|-----|---------|-------------|
| `size` | `0.1` if no `aspect` | Bar height, fraction of the canvas height. |
| `aspect` | — | Make the picture between the bars this aspect ratio (e.g. `2.39`); ignored when the canvas is already wider. With both keys, the larger bar wins. |
| `color` | `#000000` | Bar colour. |
| `progress` | `1` | Animatable 0..1: how far the bars have slid in. |

### `fill`

A full-canvas fill, for templates that paint their own backdrop over (or
instead of) the footage. Exactly one of:

* `"color"`: animatable colour, e.g. `"#101014"` or `"#00000080"`.
* `"gradient"`: `{ "kind": "linear", "angle": 90, "stops": [[0, "#00000000"], [1, "#000000CC"]] }` —
  `angle` 0 runs left → right, 90 top → bottom. Radial gradients use
  `"kind": "radial"`, `"center": [0.5, 0.5]` (fractions of the canvas) and
  `"radius": 0.75` (fraction of half the canvas diagonal).

### `vignette`

Darkens the edges and corners (precomputed once per renderer, dithered).

| Key | Default | Description |
|-----|---------|-------------|
| `strength` | `0.5` | Darkness in the corners, 0..1. |
| `radius` | `0.35` | Where darkening starts, as a fraction of the centre-to-corner distance. |
| `softness` | `0.65` | Length of the falloff, same units. |
| `color` | `#000000` | Vignette colour. |

### `grain`

Animated film grain, cheap to render: a few noise tiles are generated up front
and cycled with random offsets.

| Key | Default | Description |
|-----|---------|-------------|
| `amount` | `0.06` | Strength, 0..1. |
| `size` | `0.0012` | Grain size in s units (about 1 px at 1080p). |
| `fps` | `24` | How often the grain pattern changes per second. |
| `seed` | `1` | Pattern seed. |

### `scanlines`

Horizontal CRT lines.

| Key | Default | Description |
|-----|---------|-------------|
| `spacing` | `0.004` | Line period in s units (at least 2 px). |
| `thickness` | `0.45` | Dark part of each period, 0..1. |
| `strength` | `0.25` | Darkness of the lines, 0..1. |
| `color` | `#000000` | Line colour. |
| `speed` | `0` | Roll speed in canvas heights per second (positive moves the lines down). |

### `sweep`

A soft diagonal band of light crossing the whole canvas (for a highlight
limited to the letters, use the text layer's `sweep`).

| Key | Default | Description |
|-----|---------|-------------|
| `progress` | required | Animatable 0..1: band position from the left edge to past the right. |
| `angle` | `20` | Tilt from vertical, degrees. |
| `width` | `0.4` | Band width in s units. |
| `color` | `#FFFFFF` | Light colour. |
| `intensity` | `0.15` | Peak opacity of the band. |

### `particles`

Deterministic drifting particles; the same seed gives the same motion.

| Key | Default | Description |
|-----|---------|-------------|
| `kind` | required | `bokeh` (soft out-of-focus discs with a brighter rim), `dust` (tiny specks) or `confetti` (tumbling paper rectangles). |
| `count` | `30` | Number of particles (up to 2000). |
| `seed` | `1` | Random seed. |
| `size` | per kind | `[min, max]` in s units (bokeh `[0.015, 0.06]`, dust `[0.002, 0.006]`, confetti `[0.008, 0.016]`). |
| `colors` | `["#FFFFFF"]` | Palette; each particle picks one. |
| `velocity` | per kind | `[vx, vy]` drift in s units per second (bokeh drifts up, confetti falls). |
| `spread` | `0.01` | Random variation of the velocity. |
| `softness` | `0.6` | Edge softness of bokeh and dust discs, 0..1. |
| `twinkle` | `0.3` | How much each particle's brightness pulses, 0..1. |
| `area` | `[0, 0, 1, 1]` | Region `[x0, y0, x1, y1]` (canvas fractions) particles move in; they wrap around its edges. |

### `noise_band`

A VHS tracking-error band: a horizontal strip of bright and dark streaks that
changes every frame.

| Key | Default | Description |
|-----|---------|-------------|
| `y` | required | Animatable band centre, fraction of the canvas height (key it from below 0 to above 1 to roll it through). |
| `height` | `0.06` | Band height, fraction of the canvas height. |
| `intensity` | `0.35` | Streak strength. |
| `seed` | `1` | Pattern seed. |

## Errors

Problems are reported as `Error::Template` with the file and, where possible,
the layer: invalid JSON (with line and column), unknown keys or layer types,
values out of range, keyframes out of order, undeclared fields or stacks,
unknown `width_of` targets, missing fonts or font families, a missing sound
file. `TitleRenderer::new` additionally fails for a missing required field and
for unsupported canvas sizes or frame rates.

## Previewing

The test suite contains an ignored helper that writes PNG frames of templates,
composited over a stand-in background:

```sh
TITLE_PREVIEW_DIR=/tmp/previews TITLE_PREVIEW_TEMPLATES=clean,path/to/my-template \
  cargo test --release --lib titles::tests::export_preview_frames -- --ignored --nocapture
```

`TITLE_PREVIEW_TIMES=0.5,2.8` picks the times to render; `TITLE_PREVIEW_SETS`
chooses sample texts (`en`, `lv`, `title-only`, `ru`, `world`).
`titles::tests::bench_frame_times` prints render timings.

## Performance

Per-frame cost grows with the canvas area. Full-canvas effects (vignette,
grain, scanlines, fills) are simple per-pixel passes; text is drawn from
pre-built outlines. Blurred, glowing, revealed or swept text is drawn into a
small offscreen buffer — at reduced resolution when blurred — so even large
blur radii stay cheap. As a guide, the built-in templates render a 3840×2160
frame in about 4–12 ms on a single core of a 2025 laptop (release build), and
a 1920×1080 frame in about a quarter of that.

## Complete example

```json
{
  "format": 1,
  "name": "simple",
  "duration": 5,
  "background": { "kind": "footage", "dim": 0.5, "blur": 1.0 },
  "safe_area": { "x": 0.08, "y": 0.1 },
  "stacks": { "main": { "y": 0.5 } },
  "portrait": { "stacks": { "main": { "y": 0.45 } } },
  "fields": [
    { "name": "title", "max_chars": 60, "required": true },
    { "name": "date", "max_chars": 40 }
  ],
  "layers": [
    { "type": "vignette", "strength": 0.4 },
    {
      "type": "text", "id": "title", "field": "title",
      "font": "My Sans", "weight": 700, "size": 0.08, "max_lines": 2,
      "stack": "main",
      "ease": "ease_out",
      "dy": [[0.2, 0.03], [1.2, 0]],
      "opacity": [[0.2, 0], [1.0, 1], [3.8, 1], [4.6, 0, "ease_in_out"]],
      "shadow": { "opacity": 0.4 },
      "portrait": { "size": 0.09 }
    },
    {
      "type": "rect", "stack": "main", "gap": 0.04,
      "width_of": "title", "width_factor": 0.3, "height": 0.003,
      "reveal": [[0.8, 0], [1.6, 1, "ease_in_out"]], "reveal_from": "center",
      "opacity": [[3.8, 1], [4.6, 0]]
    },
    {
      "type": "text", "field": "date", "font": "My Sans",
      "size": 0.025, "uppercase": true, "tracking": 0.2,
      "stack": "main", "gap": 0.04, "color": "#FFFFFFB0",
      "opacity": [[1.2, 0], [2.0, 1, "ease_out"], [3.8, 1], [4.6, 0]]
    }
  ]
}
```

Put `MySans-Regular.ttf` and `MySans-Bold.ttf` (family "My Sans") in
`fonts/` next to it. Without a date the rule stays and the stack re-centres.
