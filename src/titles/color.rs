//! sRGB colours with straight (non-premultiplied) alpha.

use serde::Deserialize;
use serde::de::{self, Deserializer};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub(crate) const WHITE: Rgba = Rgba {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    pub(crate) const BLACK: Rgba = Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };

    /// Parse `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
    pub(crate) fn parse(s: &str) -> Result<Rgba, String> {
        let hex = s
            .trim()
            .strip_prefix('#')
            .ok_or_else(|| format!("colour {s:?} must start with '#'"))?;
        let digits: Vec<u8> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect::<Option<_>>()
            .ok_or_else(|| format!("colour {s:?} is not hexadecimal"))?;
        let bytes: [u8; 4] = match digits.len() {
            3 => [digits[0] * 17, digits[1] * 17, digits[2] * 17, 255],
            4 => [
                digits[0] * 17,
                digits[1] * 17,
                digits[2] * 17,
                digits[3] * 17,
            ],
            6 | 8 => {
                let mut b = [255u8; 4];
                for (i, pair) in digits.chunks(2).enumerate() {
                    b[i] = pair[0] * 16 + pair[1];
                }
                b
            }
            _ => return Err(format!("colour {s:?} must have 3, 4, 6 or 8 hex digits")),
        };
        Ok(Rgba::from_u8(bytes))
    }

    pub(crate) fn from_u8(b: [u8; 4]) -> Rgba {
        Rgba {
            r: b[0] as f32 / 255.0,
            g: b[1] as f32 / 255.0,
            b: b[2] as f32 / 255.0,
            a: b[3] as f32 / 255.0,
        }
    }

    pub(crate) fn to_u8(self) -> [u8; 3] {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [q(self.r), q(self.g), q(self.b)]
    }

    pub(crate) fn with_alpha(self, a: f32) -> Rgba {
        Rgba {
            a: self.a * a,
            ..self
        }
    }

    pub(crate) fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba(
            self.r.clamp(0.0, 1.0),
            self.g.clamp(0.0, 1.0),
            self.b.clamp(0.0, 1.0),
            self.a.clamp(0.0, 1.0),
        )
        .unwrap_or(tiny_skia::Color::TRANSPARENT)
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgba::parse(&s).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_forms() {
        assert_eq!(Rgba::parse("#fff").unwrap(), Rgba::WHITE);
        assert_eq!(Rgba::parse("#000000").unwrap(), Rgba::BLACK);
        let c = Rgba::parse("#10141880").unwrap();
        assert_eq!(c.to_u8(), [0x10, 0x14, 0x18]);
        assert!((c.a - 128.0 / 255.0).abs() < 1e-6);
        assert!(Rgba::parse("101418").is_err());
        assert!(Rgba::parse("#12345").is_err());
        assert!(Rgba::parse("#zzzzzz").is_err());
    }
}
