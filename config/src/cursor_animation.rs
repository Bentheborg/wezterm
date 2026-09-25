//! Configuration for the native cursor animation effects.
//!
//! The presets and their default values are modelled on
//! <https://github.com/sahaj-b/ghostty-cursor-shaders>
//! (MIT License, Copyright (c) 2026 Sahaj Bhatt).
use crate::RgbaColor;
use wezterm_dynamic::{FromDynamic, ToDynamic, Value};

/// Easing curves available to the cursor animations.
///
/// This is deliberately separate from `EasingFunction`: several of these
/// curves overshoot and cannot be represented by that type, and the
/// named CSS curves here are evaluated as true CSS `cubic-bezier` timing
/// functions so that they start at 0 and end at 1.
#[derive(Debug, Clone, Copy, FromDynamic, ToDynamic, PartialEq, Eq)]
pub enum CursorEasing {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    EaseOutCubic,
    EaseOutCirc,
    EaseOutBack,
    EaseOutElastic,
}

impl CursorEasing {
    /// Evaluate the curve; `x` is clamped to 0..=1.
    /// EaseOutBack and EaseOutElastic intentionally overshoot 1.0.
    pub fn evaluate(self, x: f32) -> f32 {
        let x = if x.is_nan() { 0. } else { x.clamp(0., 1.) };
        match self {
            Self::Linear => x,
            Self::Ease => css_cubic_bezier(0.25, 0.1, 0.25, 1.0, x),
            Self::EaseIn => css_cubic_bezier(0.42, 0.0, 1.0, 1.0, x),
            Self::EaseOut => css_cubic_bezier(0.0, 0.0, 0.58, 1.0, x),
            Self::EaseInOut => css_cubic_bezier(0.42, 0.0, 0.58, 1.0, x),
            Self::EaseOutCubic => 1. - (1. - x).powi(3),
            Self::EaseOutCirc => (1. - (x - 1.).powi(2)).max(0.).sqrt(),
            Self::EaseOutBack => {
                const C1: f32 = 1.70158;
                const C3: f32 = C1 + 1.;
                1. + C3 * (x - 1.).powi(3) + C1 * (x - 1.).powi(2)
            }
            Self::EaseOutElastic => {
                if x == 0. || x == 1. {
                    x
                } else {
                    const C4: f32 = (2. * std::f32::consts::PI) / 3.;
                    2f32.powf(-10. * x) * ((x * 10. - 0.75) * C4).sin() + 1.
                }
            }
        }
    }
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` evaluated at time `x`.
/// x1 and x2 are within 0..=1 for all callers, so x(t) is monotonic
/// and bisection converges.
fn css_cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    fn bez(p1: f32, p2: f32, t: f32) -> f32 {
        let u = 1. - t;
        3. * u * u * t * p1 + 3. * u * t * t * p2 + t * t * t
    }
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if bez(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez(y1, y2, (lo + hi) / 2.)
}

#[derive(Debug, Clone, Copy, FromDynamic, ToDynamic, PartialEq, Eq, Default)]
pub enum CursorMotionPreset {
    None,
    #[default]
    Tail,
    Warp,
    Sweep,
}

#[derive(Debug, Clone, Copy, FromDynamic, ToDynamic, PartialEq, Eq, Default)]
pub enum CursorModeChangePreset {
    #[default]
    None,
    Ripple,
    SonicBoom,
    RectangleBoom,
    RectangleRipple,
}

#[derive(Debug, Clone, FromDynamic, ToDynamic, PartialEq)]
pub struct CursorAnimation {
    #[dynamic(default)]
    pub enabled: bool,
    /// Frame rate used while a cursor effect is running.
    /// None inherits `animation_fps`.
    #[dynamic(default, validate = "validate_fps")]
    pub fps: Option<u16>,
    #[dynamic(default)]
    pub motion: CursorMotion,
    #[dynamic(default)]
    pub mode_change: CursorModeChange,
}

#[derive(Debug, Clone, FromDynamic, ToDynamic, PartialEq)]
pub struct CursorMotion {
    #[dynamic(default)]
    pub preset: CursorMotionPreset,
    /// None selects the preset default
    #[dynamic(default, validate = "validate_duration")]
    pub duration_ms: Option<u64>,
    #[dynamic(default)]
    pub easing: Option<CursorEasing>,
    /// Minimum distance between the old and new cursor centers,
    /// measured in cell heights, before an effect is shown.
    #[dynamic(default = "default_min_distance", validate = "validate_non_negative")]
    pub min_distance: f32,
    /// None uses the effective cursor color
    #[dynamic(default)]
    pub color: Option<RgbaColor>,
    /// Edge softness in pixels; None selects the preset default
    #[dynamic(default, validate = "validate_opt_non_negative")]
    pub blur: Option<f32>,
    #[dynamic(default)]
    pub tail: CursorTail,
    #[dynamic(default)]
    pub warp: CursorWarp,
    #[dynamic(default)]
    pub sweep: CursorSweep,
}

#[derive(Debug, Clone, FromDynamic, ToDynamic, PartialEq)]
pub struct CursorTail {
    #[dynamic(default = "default_tail_max_length", validate = "validate_positive")]
    pub max_length: f32,
}

#[derive(Debug, Clone, FromDynamic, ToDynamic, PartialEq)]
pub struct CursorWarp {
    #[dynamic(default = "default_warp_trail_size", validate = "validate_unit")]
    pub trail_size: f32,
    #[dynamic(default = "default_one", validate = "validate_non_negative")]
    pub thickness: f32,
    #[dynamic(
        default = "default_warp_thickness_x",
        validate = "validate_non_negative"
    )]
    pub thickness_x: f32,
    #[dynamic(default)]
    pub fade: bool,
    #[dynamic(
        default = "default_warp_fade_exponent",
        validate = "validate_non_negative"
    )]
    pub fade_exponent: f32,
}

#[derive(Debug, Clone, FromDynamic, ToDynamic, PartialEq)]
pub struct CursorSweep {
    #[dynamic(default = "default_sweep_trail_length", validate = "validate_positive")]
    pub trail_length: f32,
}

#[derive(Debug, Clone, FromDynamic, ToDynamic, PartialEq)]
pub struct CursorModeChange {
    #[dynamic(default)]
    pub preset: CursorModeChangePreset,
    #[dynamic(default, validate = "validate_duration")]
    pub duration_ms: Option<u64>,
    #[dynamic(default)]
    pub easing: Option<CursorEasing>,
    #[dynamic(default)]
    pub color: Option<RgbaColor>,
    #[dynamic(default, validate = "validate_opt_non_negative")]
    pub blur: Option<f32>,
    /// Added to the animation progress; positive values skip the start
    #[dynamic(default, validate = "validate_finite")]
    pub animation_start_offset: f32,
    /// Ripple / SonicBoom radius; None selects the preset default
    #[dynamic(default, validate = "validate_opt_non_negative")]
    pub max_radius: Option<f32>,
    #[dynamic(default = "default_max_size", validate = "validate_non_negative")]
    pub max_size: f32,
    #[dynamic(default = "default_ring_thickness", validate = "validate_non_negative")]
    pub ring_thickness: f32,
}

/// Build the value that an empty lua table would produce, so that
/// `Default` and the per-field dynamic defaults cannot drift apart.
fn from_empty<T: FromDynamic>() -> T {
    T::from_dynamic(&Value::Object(Default::default()), Default::default())
        .expect("defaults are valid")
}

macro_rules! default_from_empty {
    ($($t:ty),*) => {$(
        impl Default for $t {
            fn default() -> Self {
                from_empty()
            }
        }
    )*};
}
default_from_empty!(
    CursorAnimation,
    CursorMotion,
    CursorTail,
    CursorWarp,
    CursorSweep,
    CursorModeChange
);

impl CursorMotion {
    pub fn effective_duration_ms(&self) -> u64 {
        self.duration_ms.unwrap_or(match self.preset {
            CursorMotionPreset::Tail => 90,
            _ => 200,
        })
    }

    pub fn effective_easing(&self) -> CursorEasing {
        self.easing.unwrap_or(match self.preset {
            CursorMotionPreset::Sweep => CursorEasing::EaseOutCubic,
            _ => CursorEasing::EaseOutCirc,
        })
    }

    pub fn effective_blur(&self) -> f32 {
        self.blur.unwrap_or(match self.preset {
            CursorMotionPreset::Warp => 1.0,
            _ => 2.0,
        })
    }
}

impl CursorModeChange {
    pub fn effective_duration_ms(&self) -> u64 {
        self.duration_ms.unwrap_or(150)
    }

    pub fn effective_easing(&self) -> CursorEasing {
        self.easing.unwrap_or(CursorEasing::EaseOutCirc)
    }

    pub fn effective_blur(&self) -> f32 {
        self.blur.unwrap_or(match self.preset {
            CursorModeChangePreset::RectangleRipple => 1.0,
            _ => 3.0,
        })
    }

    pub fn effective_max_radius(&self) -> f32 {
        self.max_radius.unwrap_or(match self.preset {
            CursorModeChangePreset::SonicBoom => 0.06,
            _ => 0.05,
        })
    }
}

fn default_min_distance() -> f32 {
    1.5
}
fn default_tail_max_length() -> f32 {
    0.2
}
fn default_warp_trail_size() -> f32 {
    0.8
}
fn default_one() -> f32 {
    1.0
}
fn default_warp_thickness_x() -> f32 {
    0.9
}
fn default_warp_fade_exponent() -> f32 {
    5.0
}
fn default_sweep_trail_length() -> f32 {
    0.5
}
fn default_max_size() -> f32 {
    0.05
}
fn default_ring_thickness() -> f32 {
    0.02
}

fn validate_fps(value: &Option<u16>) -> Result<(), String> {
    match value {
        Some(fps) if !(1..=480).contains(fps) => {
            Err(format!("fps must be in the range 1-480, got {}", fps))
        }
        _ => Ok(()),
    }
}

fn validate_duration(value: &Option<u64>) -> Result<(), String> {
    match value {
        Some(0) => Err("duration_ms must be greater than zero".to_string()),
        Some(ms) if *ms > 10_000 => Err(format!("duration_ms must be at most 10000, got {}", ms)),
        _ => Ok(()),
    }
}

fn validate_finite(value: &f32) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!("{} is not a finite number", value))
    }
}

fn validate_non_negative(value: &f32) -> Result<(), String> {
    validate_finite(value)?;
    if *value < 0. {
        Err(format!("{} must not be negative", value))
    } else {
        Ok(())
    }
}

fn validate_opt_non_negative(value: &Option<f32>) -> Result<(), String> {
    value.as_ref().map_or(Ok(()), validate_non_negative)
}

fn validate_positive(value: &f32) -> Result<(), String> {
    validate_non_negative(value)?;
    if *value == 0. {
        Err("value must be greater than zero".to_string())
    } else {
        Ok(())
    }
}

fn validate_unit(value: &f32) -> Result<(), String> {
    validate_non_negative(value)?;
    if *value > 1. {
        Err(format!("{} must be in the range 0.0-1.0", value))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use wezterm_dynamic::Object;

    fn obj(pairs: &[(&str, Value)]) -> Value {
        let mut o = Object::default();
        for (k, v) in pairs {
            o.insert(Value::String(k.to_string()), v.clone());
        }
        Value::Object(o)
    }

    fn s(v: &str) -> Value {
        Value::String(v.to_string())
    }

    fn parse(v: Value) -> Result<CursorAnimation, wezterm_dynamic::Error> {
        CursorAnimation::from_dynamic(&v, Default::default())
    }

    #[test]
    fn defaults() {
        let c = CursorAnimation::default();
        assert!(!c.enabled);
        assert_eq!(c.fps, None);
        assert_eq!(c.motion.preset, CursorMotionPreset::Tail);
        assert_eq!(c.motion.effective_duration_ms(), 90);
        assert_eq!(c.motion.effective_easing(), CursorEasing::EaseOutCirc);
        assert_eq!(c.motion.effective_blur(), 2.0);
        assert_eq!(c.motion.min_distance, 1.5);
        assert_eq!(c.motion.tail.max_length, 0.2);
        assert_eq!(c.motion.warp.trail_size, 0.8);
        assert!(!c.motion.warp.fade);
        assert_eq!(c.motion.sweep.trail_length, 0.5);
        assert_eq!(c.mode_change.preset, CursorModeChangePreset::None);
        assert_eq!(c.mode_change.effective_duration_ms(), 150);
        assert_eq!(c.mode_change.ring_thickness, 0.02);
    }

    #[test]
    fn preset_defaults() {
        let warp = parse(obj(&[("motion", obj(&[("preset", s("Warp"))]))])).unwrap();
        assert_eq!(warp.motion.effective_duration_ms(), 200);
        assert_eq!(warp.motion.effective_blur(), 1.0);
        assert_eq!(warp.motion.effective_easing(), CursorEasing::EaseOutCirc);

        let sweep = parse(obj(&[("motion", obj(&[("preset", s("Sweep"))]))])).unwrap();
        assert_eq!(sweep.motion.effective_duration_ms(), 200);
        assert_eq!(sweep.motion.effective_easing(), CursorEasing::EaseOutCubic);

        for (name, radius, blur) in [
            ("Ripple", 0.05, 3.0),
            ("SonicBoom", 0.06, 3.0),
            ("RectangleBoom", 0.05, 3.0),
            ("RectangleRipple", 0.05, 1.0),
        ] {
            let c = parse(obj(&[("mode_change", obj(&[("preset", s(name))]))])).unwrap();
            assert_eq!(c.mode_change.effective_max_radius(), radius, "{}", name);
            assert_eq!(c.mode_change.effective_blur(), blur, "{}", name);
        }
    }

    #[test]
    fn parse_full() {
        let c = parse(obj(&[
            ("enabled", Value::Bool(true)),
            ("fps", Value::U64(120)),
            (
                "motion",
                obj(&[
                    ("preset", s("Warp")),
                    ("duration_ms", Value::U64(150)),
                    ("easing", s("EaseOutBack")),
                    ("min_distance", Value::F64(1.0.into())),
                    ("color", s("#ff0000")),
                    (
                        "warp",
                        obj(&[
                            ("fade", Value::Bool(true)),
                            ("fade_exponent", Value::U64(3)),
                        ]),
                    ),
                ]),
            ),
            (
                "mode_change",
                obj(&[
                    ("preset", s("Ripple")),
                    ("max_radius", Value::F64(0.026.into())),
                    ("animation_start_offset", Value::F64(0.01.into())),
                ]),
            ),
        ]))
        .unwrap();
        assert!(c.enabled);
        assert_eq!(c.fps, Some(120));
        assert_eq!(c.motion.preset, CursorMotionPreset::Warp);
        assert_eq!(c.motion.effective_duration_ms(), 150);
        assert_eq!(c.motion.effective_easing(), CursorEasing::EaseOutBack);
        assert!(c.motion.color.is_some());
        assert!(c.motion.warp.fade);
        assert_eq!(c.motion.warp.fade_exponent, 3.0);
        assert_eq!(c.mode_change.preset, CursorModeChangePreset::Ripple);
        assert!((c.mode_change.effective_max_radius() - 0.026).abs() < 1e-6);
    }

    #[test]
    fn invalid_values() {
        for bad in [
            obj(&[("fps", Value::U64(0))]),
            obj(&[("motion", obj(&[("duration_ms", Value::U64(0))]))]),
            obj(&[(
                "motion",
                obj(&[("min_distance", Value::F64((-1.0).into()))]),
            )]),
            obj(&[("motion", obj(&[("blur", Value::F64((-1.0).into()))]))]),
            obj(&[(
                "motion",
                obj(&[("tail", obj(&[("max_length", Value::U64(0))]))]),
            )]),
            obj(&[(
                "motion",
                obj(&[("sweep", obj(&[("trail_length", Value::F64((-0.5).into()))]))]),
            )]),
            obj(&[(
                "motion",
                obj(&[("warp", obj(&[("thickness", Value::F64((-1.0).into()))]))]),
            )]),
            obj(&[(
                "mode_change",
                obj(&[("ring_thickness", Value::F64((-0.1).into()))]),
            )]),
            obj(&[("motion", obj(&[("preset", s("Bogus"))]))]),
        ] {
            assert!(parse(bad.clone()).is_err(), "{:?} should be rejected", bad);
        }
    }

    #[test]
    fn easing_endpoints_and_finite() {
        use CursorEasing::*;
        for e in [
            Linear,
            Ease,
            EaseIn,
            EaseOut,
            EaseInOut,
            EaseOutCubic,
            EaseOutCirc,
            EaseOutBack,
            EaseOutElastic,
        ] {
            assert!(e.evaluate(0.).abs() < 1e-3, "{:?}(0)", e);
            assert!((e.evaluate(1.) - 1.).abs() < 1e-3, "{:?}(1)", e);
            for i in 0..=100 {
                assert!(e.evaluate(i as f32 / 100.).is_finite());
            }
            assert!(e.evaluate(f32::NAN).is_finite());
            assert!(e.evaluate(5.).is_finite());
        }
    }

    #[test]
    fn easing_monotonic() {
        use CursorEasing::*;
        for e in [
            Linear,
            Ease,
            EaseIn,
            EaseOut,
            EaseInOut,
            EaseOutCubic,
            EaseOutCirc,
        ] {
            let mut prev = e.evaluate(0.);
            for i in 1..=200 {
                let v = e.evaluate(i as f32 / 200.);
                assert!(v >= prev - 1e-5, "{:?} not monotonic at {}", e, i);
                prev = v;
            }
        }
    }

    #[test]
    fn easing_overshoot() {
        for e in [CursorEasing::EaseOutBack, CursorEasing::EaseOutElastic] {
            let max = (0..=200)
                .map(|i| e.evaluate(i as f32 / 200.))
                .fold(f32::MIN, f32::max);
            assert!(max > 1.01, "{:?} should overshoot, max {}", e, max);
        }
    }
}
