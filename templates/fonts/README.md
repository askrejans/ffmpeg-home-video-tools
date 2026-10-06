# Bundled fonts

These fonts are shared by the built-in title templates (`templates/clean`,
`templates/cinematic`, `templates/retro`) and are embedded in the library when
the `builtin-templates` feature is enabled. All of them are licensed under the
SIL Open Font License, Version 1.1; each family's licence text is in the
`OFL-*.txt` file next to it. The files are unmodified static (non-variable)
TrueType fonts.

| File | Family / weight | Coverage | Licence | Source |
|------|-----------------|----------|---------|--------|
| `IBMPlexSans-Light.ttf` | IBM Plex Sans 300 | Latin (incl. Latin Extended), Cyrillic, Greek | OFL 1.1, Reserved Font Name "Plex" — `OFL-IBMPlexSans.txt` | <https://github.com/IBM/plex> (`packages/plex-sans/fonts/complete/ttf/`) |
| `IBMPlexSans-Regular.ttf` | IBM Plex Sans 400 | Latin (incl. Latin Extended), Cyrillic, Greek | OFL 1.1 — `OFL-IBMPlexSans.txt` | <https://github.com/IBM/plex> |
| `IBMPlexSans-SemiBold.ttf` | IBM Plex Sans 600 | Latin (incl. Latin Extended), Cyrillic, Greek | OFL 1.1 — `OFL-IBMPlexSans.txt` | <https://github.com/IBM/plex> |
| `NotoSerifDisplay-Light.ttf` | Noto Serif Display 300 | Latin (incl. Latin Extended), Cyrillic, Greek | OFL 1.1 — `OFL-NotoSerifDisplay.txt` | <https://github.com/notofonts/notofonts.github.io> (`fonts/NotoSerifDisplay/unhinted/ttf/`), project <https://github.com/notofonts/latin-greek-cyrillic> |
| `Play-Regular.ttf` | Play 400 | Latin (incl. Latin Extended), Cyrillic, Greek | OFL 1.1, Reserved Font Names "Play", "Playtype", "Playtype Sans" — `OFL-Play.txt` | <https://github.com/google/fonts/tree/main/ofl/play> |
| `Play-Bold.ttf` | Play 700 | Latin (incl. Latin Extended), Cyrillic, Greek | OFL 1.1 — `OFL-Play.txt` | <https://github.com/google/fonts/tree/main/ofl/play> |
| `VT323-Regular.ttf` | VT323 400 | Latin (incl. Latin Extended) only | OFL 1.1 — `OFL-VT323.txt` | <https://github.com/google/fonts/tree/main/ofl/vt323> |

VT323 is only used for the on-screen-display decoration of the `retro`
template (the "PLAY" label and the date stamp). Characters it lacks (Cyrillic,
Greek, …) fall back to Play, the template's declared fallback font.

Total payload: about 1.6 MB.

The OFL allows these fonts to be bundled, embedded and redistributed with
software, including commercial software, provided the fonts are not sold on
their own and the licence travels with them. Modified versions may not use the
reserved font names.
