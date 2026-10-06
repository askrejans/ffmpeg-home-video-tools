//! Keyframed values and easing curves.

use super::color::Rgba;
use serde::Deserialize;
use serde::de::{self, Deserializer};
use serde_json::Value;

/// Timing curve applied to the segment that ends at a keyframe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Easing {
    Linear,
    /// Hold the previous value until the keyframe, then jump.
    Step,
    EaseIn,
    EaseOut,
    EaseInOut,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
    EaseInExpo,
    EaseOutExpo,
    EaseInBack,
    EaseOutBack,
    EaseInOutSine,
    Bezier(f32, f32, f32, f32),
}

impl Easing {
    pub(crate) fn parse(v: &Value) -> Result<Self, String> {
        match v {
            Value::String(s) => Self::parse_str(s),
            Value::Array(a) if a.len() == 4 => {
                let mut n = [0f32; 4];
                for (slot, item) in n.iter_mut().zip(a) {
                    *slot = item
                        .as_f64()
                        .ok_or_else(|| "cubic-bezier control points must be numbers".to_string())?
                        as f32;
                }
                Self::bezier(n)
            }
            _ => Err(format!(
                "invalid easing {v}; use a name like \"ease_out\" or [x1, y1, x2, y2]"
            )),
        }
    }

    fn bezier(n: [f32; 4]) -> Result<Self, String> {
        if !n.iter().all(|v| v.is_finite())
            || !(0.0..=1.0).contains(&n[0])
            || !(0.0..=1.0).contains(&n[2])
        {
            return Err("cubic-bezier x control points must be within 0..1".into());
        }
        Ok(Easing::Bezier(n[0], n[1], n[2], n[3]))
    }

    fn parse_str(s: &str) -> Result<Self, String> {
        let norm = s.trim().to_ascii_lowercase().replace('-', "_");
        if let Some(inner) = norm
            .strip_prefix("cubic_bezier(")
            .and_then(|r| r.strip_suffix(')'))
        {
            let parts: Vec<f32> = inner
                .split(',')
                .map(|p| p.trim().parse::<f32>())
                .collect::<Result<_, _>>()
                .map_err(|_| format!("invalid cubic-bezier easing {s:?}"))?;
            if parts.len() != 4 {
                return Err(format!("cubic-bezier needs four numbers: {s:?}"));
            }
            return Self::bezier([parts[0], parts[1], parts[2], parts[3]]);
        }
        Ok(match norm.as_str() {
            "linear" => Easing::Linear,
            "step" | "hold" => Easing::Step,
            "ease_in" | "ease_in_cubic" => Easing::EaseIn,
            "ease_out" | "ease_out_cubic" => Easing::EaseOut,
            "ease_in_out" | "ease_in_out_cubic" | "ease" => Easing::EaseInOut,
            "ease_in_quad" => Easing::EaseInQuad,
            "ease_out_quad" => Easing::EaseOutQuad,
            "ease_in_out_quad" => Easing::EaseInOutQuad,
            "ease_in_expo" => Easing::EaseInExpo,
            "ease_out_expo" => Easing::EaseOutExpo,
            "ease_in_back" => Easing::EaseInBack,
            "ease_out_back" => Easing::EaseOutBack,
            "ease_in_out_sine" => Easing::EaseInOutSine,
            _ => {
                return Err(format!(
                    "unknown easing {s:?} (expected linear, step, ease_in, ease_out, ease_in_out, \
                     ease_in_quad, ease_out_quad, ease_in_out_quad, ease_in_expo, ease_out_expo, \
                     ease_in_back, ease_out_back, ease_in_out_sine or cubic-bezier(x1,y1,x2,y2))"
                ));
            }
        })
    }

    /// Map linear progress `x` in 0..1 to eased progress.
    pub(crate) fn apply(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Easing::Linear => x,
            Easing::Step => {
                if x >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Easing::EaseIn => x * x * x,
            Easing::EaseOut => 1.0 - (1.0 - x).powi(3),
            Easing::EaseInOut => {
                if x < 0.5 {
                    4.0 * x * x * x
                } else {
                    1.0 - (-2.0 * x + 2.0).powi(3) / 2.0
                }
            }
            Easing::EaseInQuad => x * x,
            Easing::EaseOutQuad => 1.0 - (1.0 - x) * (1.0 - x),
            Easing::EaseInOutQuad => {
                if x < 0.5 {
                    2.0 * x * x
                } else {
                    1.0 - (-2.0 * x + 2.0).powi(2) / 2.0
                }
            }
            Easing::EaseInExpo => {
                if x <= 0.0 {
                    0.0
                } else {
                    (2f32).powf(10.0 * x - 10.0)
                }
            }
            Easing::EaseOutExpo => {
                if x >= 1.0 {
                    1.0
                } else {
                    1.0 - (2f32).powf(-10.0 * x)
                }
            }
            Easing::EaseInBack => {
                const C1: f32 = 1.70158;
                const C3: f32 = C1 + 1.0;
                C3 * x * x * x - C1 * x * x
            }
            Easing::EaseOutBack => {
                const C1: f32 = 1.70158;
                const C3: f32 = C1 + 1.0;
                1.0 + C3 * (x - 1.0).powi(3) + C1 * (x - 1.0).powi(2)
            }
            Easing::EaseInOutSine => -((std::f32::consts::PI * x).cos() - 1.0) / 2.0,
            Easing::Bezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, x),
        }
    }
}

/// CSS-style cubic-bezier timing function.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let (x1, y1, x2, y2) = (x1 as f64, y1 as f64, x2 as f64, y2 as f64);
    let target = x as f64;
    let bez = |a: f64, b: f64, t: f64| {
        let mt = 1.0 - t;
        3.0 * mt * mt * t * a + 3.0 * mt * t * t * b + t * t * t
    };
    let dbez = |a: f64, b: f64, t: f64| {
        let mt = 1.0 - t;
        3.0 * mt * mt * a + 6.0 * mt * t * (b - a) + 3.0 * t * t * (1.0 - b)
    };
    // Newton iterations, falling back to bisection when the slope is flat.
    let mut t = target;
    for _ in 0..8 {
        let err = bez(x1, x2, t) - target;
        if err.abs() < 1e-6 {
            return bez(y1, y2, t) as f32;
        }
        let d = dbez(x1, x2, t);
        if d.abs() < 1e-6 {
            break;
        }
        t = (t - err / d).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    t = target;
    for _ in 0..40 {
        let v = bez(x1, x2, t);
        if (v - target).abs() < 1e-7 {
            break;
        }
        if v < target {
            lo = t;
        } else {
            hi = t;
        }
        t = (lo + hi) / 2.0;
    }
    bez(y1, y2, t) as f32
}

/// A value that can be interpolated between keyframes.
pub(crate) trait Lerp: Copy {
    fn lerp(a: Self, b: Self, t: f32) -> Self;
    fn parse(v: &Value) -> Result<Self, String>;
}

impl Lerp for f32 {
    fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }
    fn parse(v: &Value) -> Result<f32, String> {
        let n = v
            .as_f64()
            .ok_or_else(|| format!("expected a number, found {v}"))?;
        if !n.is_finite() {
            return Err("numbers must be finite".into());
        }
        Ok(n as f32)
    }
}

impl Lerp for Rgba {
    fn lerp(a: Rgba, b: Rgba, t: f32) -> Rgba {
        Rgba {
            r: f32::lerp(a.r, b.r, t),
            g: f32::lerp(a.g, b.g, t),
            b: f32::lerp(a.b, b.b, t),
            a: f32::lerp(a.a, b.a, t),
        }
    }
    fn parse(v: &Value) -> Result<Rgba, String> {
        match v {
            Value::String(s) => Rgba::parse(s),
            _ => Err(format!(
                "expected a colour string like \"#ffffff\", found {v}"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Key<T> {
    pub t: f64,
    pub v: T,
    pub ease: Easing,
}

/// A constant or keyframed property.
#[derive(Debug, Clone)]
pub(crate) enum Anim<T> {
    Const(T),
    Keys(Vec<Key<T>>),
}

impl<T: Lerp> Anim<T> {
    pub(crate) fn at(&self, t: f64) -> T {
        match self {
            Anim::Const(v) => *v,
            Anim::Keys(keys) => {
                let first = &keys[0];
                if t <= first.t {
                    return first.v;
                }
                let last = &keys[keys.len() - 1];
                if t >= last.t {
                    return last.v;
                }
                // Keys are few; a linear scan is fastest in practice.
                let i = keys.iter().position(|k| k.t > t).unwrap_or(keys.len() - 1);
                let (a, b) = (&keys[i - 1], &keys[i]);
                let span = b.t - a.t;
                let x = if span <= 0.0 {
                    1.0
                } else {
                    ((t - a.t) / span) as f32
                };
                T::lerp(a.v, b.v, b.ease.apply(x))
            }
        }
    }

    pub(crate) fn values(&self) -> Vec<T> {
        match self {
            Anim::Const(v) => vec![*v],
            Anim::Keys(keys) => keys.iter().map(|k| k.v).collect(),
        }
    }

    pub(crate) fn parse(v: &Value) -> Result<Self, String> {
        match v {
            Value::Array(items) => Self::parse_keys(items, Easing::Linear),
            Value::Object(map) => {
                for k in map.keys() {
                    if k != "keys" && k != "ease" {
                        return Err(format!(
                            "unknown key {k:?} in animated value (expected \"keys\" and \"ease\")"
                        ));
                    }
                }
                let ease = match map.get("ease") {
                    Some(e) => Easing::parse(e)?,
                    None => Easing::Linear,
                };
                match map.get("keys") {
                    Some(Value::Array(items)) => Self::parse_keys(items, ease),
                    _ => Err("animated value object needs a \"keys\" array".into()),
                }
            }
            other => Ok(Anim::Const(T::parse(other)?)),
        }
    }

    fn parse_keys(items: &[Value], default_ease: Easing) -> Result<Self, String> {
        if items.is_empty() {
            return Err("keyframe list is empty".into());
        }
        let mut keys = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let arr = item.as_array().filter(|a| a.len() == 2 || a.len() == 3).ok_or_else(|| {
                format!("keyframe {i}: expected [time, value] or [time, value, easing], found {item}")
            })?;
            let t = arr[0]
                .as_f64()
                .filter(|t| t.is_finite())
                .ok_or_else(|| format!("keyframe {i}: time must be a number of seconds"))?;
            let v = T::parse(&arr[1]).map_err(|e| format!("keyframe {i}: {e}"))?;
            let ease = match arr.get(2) {
                Some(e) => Easing::parse(e).map_err(|e| format!("keyframe {i}: {e}"))?,
                None => default_ease,
            };
            keys.push(Key { t, v, ease });
        }
        if keys.windows(2).any(|w| w[1].t < w[0].t) {
            return Err("keyframe times must be in ascending order".into());
        }
        if keys.len() == 1 {
            return Ok(Anim::Const(keys[0].v));
        }
        Ok(Anim::Keys(keys))
    }
}

impl<'de, T: Lerp> Deserialize<'de> for Anim<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        Anim::parse(&v).map_err(de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for Easing {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        Easing::parse(&v).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn easing_endpoints() {
        for e in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::EaseOutBack,
            Easing::EaseOutExpo,
            Easing::Bezier(0.2, 0.8, 0.2, 1.0),
        ] {
            assert!(e.apply(0.0).abs() < 1e-4, "{e:?}");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-4, "{e:?}");
        }
        assert!(Easing::EaseOutBack.apply(0.7) > 1.0);
        let css_ease = Easing::parse(&json!("cubic-bezier(0.25, 0.1, 0.25, 1)")).unwrap();
        assert!((css_ease.apply(0.5) - 0.8024).abs() < 0.01);
    }

    #[test]
    fn keyframes_interpolate() {
        let a: Anim<f32> = Anim::parse(&json!([[0, 0], [1, 10], [2, 10, "step"], [3, 0]])).unwrap();
        assert_eq!(a.at(-1.0), 0.0);
        assert!((a.at(0.5) - 5.0).abs() < 1e-5);
        assert_eq!(a.at(1.5), 10.0);
        assert!((a.at(2.5) - 5.0).abs() < 1e-5);
        assert_eq!(a.at(9.0), 0.0);
        let b: Anim<f32> =
            Anim::parse(&json!({"keys": [[0, 0], [1, 1]], "ease": "ease_in"})).unwrap();
        assert!((b.at(0.5) - 0.125).abs() < 1e-5);
        assert!(Anim::<f32>::parse(&json!([[1, 0], [0, 1]])).is_err());
        assert!(Anim::<f32>::parse(&json!([[0, 0, "bogus"]])).is_err());
    }
}
