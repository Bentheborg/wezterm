//! Native cursor animation effects: a motion effect (Tail, Warp, Sweep)
//! drawn when the cursor jumps, and a mode-change effect (Ripple,
//! SonicBoom, RectangleBoom, RectangleRipple) drawn when the cursor
//! shape changes.
//!
//! The effects are independent reimplementations, as plain geometry, of
//! the GLSL shaders in <https://github.com/sahaj-b/ghostty-cursor-shaders>
//! (MIT License, Copyright (c) 2026 Sahaj Bhatt).  Instead of evaluating
//! a signed distance field per pixel we emit a few convex polygons with
//! per-vertex alpha, and approximate the shaders' `BLUR` with a linear
//! alpha falloff strip around each shape.
//!
//! All coordinates in this module are window pixels with the origin at
//! the top left and y pointing down.
use crate::quad::{
    TripleLayerQuadAllocator, TripleLayerQuadAllocatorTrait, Vertex, IS_SOLID_COLOR,
};
use ::window::DeadKeyStatus;
use config::{CursorAnimation, CursorModeChangePreset, CursorMotionPreset};
use mux::tab::PositionedPane;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};
use termwiz::surface::CursorShape;
use window::bitmaps::TextureRect;
use window::color::LinearRgba;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    fn right(&self) -> f32 {
        self.x + self.w
    }
    fn bottom(&self) -> f32 {
        self.y + self.h
    }
    fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2., self.y + self.h / 2.)
    }
    fn is_valid(&self) -> bool {
        [self.x, self.y, self.w, self.h]
            .iter()
            .all(|v| v.is_finite())
            && self.w >= 0.
            && self.h >= 0.
    }
}

/// The cursor shape, ignoring blinking
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorKind {
    Block,
    Bar,
    Underline,
}

impl CursorKind {
    pub fn from_shape(shape: CursorShape) -> Self {
        match shape {
            CursorShape::BlinkingBar | CursorShape::SteadyBar => Self::Bar,
            CursorShape::BlinkingUnderline | CursorShape::SteadyUnderline => Self::Underline,
            CursorShape::Default | CursorShape::BlinkingBlock | CursorShape::SteadyBlock => {
                Self::Block
            }
        }
    }
}

/// The cursor exactly as `render_screen_line` drew it
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderedCursor {
    /// The visible part of the cursor: the whole cell(s) for a block,
    /// the stroke for bar and underline cursors.
    pub rect: Rect,
    pub kind: CursorKind,
    pub color: LinearRgba,
}

/// Everything the animation needs to know about the cursor in one frame
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorVisualSnapshot {
    pub rect: Rect,
    pub kind: CursorKind,
    pub color: LinearRgba,
    pub cell_height: f32,
    /// Effects are clipped to this rectangle
    pub pane_rect: Rect,
    /// The reference shaders measure lengths relative to the window height
    pub window_height: f32,
    /// Changes whenever the mapping from cells to pixels changes
    /// (resize, font, scrolling, alt screen, config reload...).
    /// A change snaps the animation instead of animating the jump.
    pub layout_key: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MotionAnim {
    from: Rect,
    to: Rect,
    start: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ModeAnim {
    rect: Rect,
    start: Instant,
}

/// What the animation sees of a pane's cursor in one frame
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CursorInput {
    /// Nothing to animate: pane inactive, window unfocused,
    /// composing, or the cursor is outside the viewport.
    Inactive,
    /// The application hid the cursor.  Programs (and ConPTY) commonly
    /// hide the cursor while they redraw, so this keeps the last visible
    /// position and the next visible cursor animates from there.
    Hidden,
    Visible(CursorVisualSnapshot),
}

/// Per-pane animation state
#[derive(Default, Debug)]
pub struct CursorAnimState {
    /// The paint generation in which this pane was last painted
    painted: Option<usize>,
    /// The last visible cursor
    previous: Option<CursorVisualSnapshot>,
    visible: bool,
    motion: Option<MotionAnim>,
    mode_change: Option<ModeAnim>,
}

impl CursorAnimState {
    /// Record that the pane is being painted in paint `generation`.
    /// A pane that was skipped (eg: its tab was not visible) may have a
    /// cursor that moved while hidden, so it starts afresh.
    pub fn mark_painted(&mut self, generation: usize) {
        if let Some(last) = self.painted {
            if last != generation && last + 1 != generation {
                *self = Self::default();
            }
        }
        self.painted = Some(generation);
    }

    /// Feed the current cursor state
    pub fn update(&mut self, input: CursorInput, now: Instant, config: &CursorAnimation) {
        let snap = match input {
            CursorInput::Hidden => {
                self.visible = false;
                return;
            }
            CursorInput::Visible(snap) if snap.rect.is_valid() && snap.pane_rect.is_valid() => snap,
            _ => {
                *self = Self::default();
                return;
            }
        };
        self.visible = true;

        match self.previous {
            Some(prev) if prev.layout_key == snap.layout_key => {
                if prev.kind != snap.kind
                    && config.mode_change.preset != CursorModeChangePreset::None
                {
                    self.mode_change = Some(ModeAnim {
                        rect: snap.rect,
                        start: now,
                    });
                }
                if prev.rect != snap.rect {
                    let (px, py) = prev.rect.center();
                    let (cx, cy) = snap.rect.center();
                    let dist = (cx - px).hypot(cy - py);
                    self.motion = if config.motion.preset != CursorMotionPreset::None
                        && dist > config.motion.min_distance * snap.cell_height
                    {
                        Some(MotionAnim {
                            from: prev.rect,
                            to: snap.rect,
                            start: now,
                        })
                    } else {
                        None
                    };
                }
            }
            _ => {
                self.motion = None;
                self.mode_change = None;
            }
        }
        self.previous = Some(snap);
    }

    /// True if an effect should be drawn this frame
    pub fn is_animating(&self) -> bool {
        self.visible && (self.motion.is_some() || self.mode_change.is_some())
    }

    /// Emit the motion effect into `tris`.
    /// Returns the time at which the effect ends, or None once it has.
    pub fn render_motion(
        &mut self,
        now: Instant,
        config: &CursorAnimation,
        tris: &mut Vec<[Pt; 3]>,
    ) -> Option<Instant> {
        let (anim, snap) = match (self.motion, self.previous) {
            (Some(anim), Some(snap)) => (anim, snap),
            _ => return None,
        };
        let motion = &config.motion;
        let duration = Duration::from_millis(motion.effective_duration_ms().max(1));
        let elapsed = now.saturating_duration_since(anim.start);
        if elapsed >= duration {
            self.motion = None;
            return None;
        }
        let progress = elapsed.as_secs_f32() / duration.as_secs_f32();
        let ease = |x: f32| motion.effective_easing().evaluate(x);
        let unit = snap.window_height / 2.;

        let mut em = Emitter {
            tris,
            clip: snap.pane_rect,
            // Punch a hole where the real cursor is, like the shaders do
            exclude: Some(snap.rect),
            fade: None,
        };
        let blur = motion.effective_blur();
        match motion.preset {
            CursorMotionPreset::None => {}
            CursorMotionPreset::Tail => {
                let poly = tail_polygon(
                    anim.from,
                    anim.to,
                    progress,
                    motion.tail.max_length * unit,
                    ease,
                );
                em.feathered(poly.as_slice(), blur);
            }
            CursorMotionPreset::Sweep => {
                let poly = sweep_polygon(
                    anim.from,
                    anim.to,
                    ease(progress),
                    motion.sweep.trail_length,
                );
                em.feathered(poly.as_slice(), blur);
            }
            CursorMotionPreset::Warp => {
                let warp = &motion.warp;
                let poly = warp_polygon(
                    anim.from,
                    anim.to,
                    elapsed.as_secs_f32(),
                    duration.as_secs_f32(),
                    warp.trail_size,
                    warp.thickness,
                    warp.thickness_x,
                    ease,
                );
                if warp.fade {
                    let (ox, oy) = anim.from.center();
                    let (tx, ty) = anim.to.center();
                    em.fade = Some(Fade {
                        ox,
                        oy,
                        dx: tx - ox,
                        dy: ty - oy,
                        exponent: warp.fade_exponent,
                    });
                }
                em.feathered(poly.as_slice(), blur);
            }
        }
        Some(anim.start + duration)
    }

    /// Emit the mode-change effect into `tris`.
    /// Returns the time at which the effect ends, or None once it has.
    pub fn render_mode_change(
        &mut self,
        now: Instant,
        config: &CursorAnimation,
        tris: &mut Vec<[Pt; 3]>,
    ) -> Option<Instant> {
        let (anim, snap) = match (self.mode_change, self.previous) {
            (Some(anim), Some(snap)) => (anim, snap),
            _ => return None,
        };
        let mode = &config.mode_change;
        let duration = mode.effective_duration_ms().max(1) as f32 / 1000.;
        let elapsed = now.saturating_duration_since(anim.start).as_secs_f32();
        let progress = elapsed / duration + mode.animation_start_offset;
        if progress.is_nan() || progress >= 1. {
            self.mode_change = None;
            return None;
        }
        let progress = progress.max(0.);
        let eased = mode.effective_easing().evaluate(progress);
        // easeOutPulse from the shaders: 1 - t(2 - t)
        let fade = (1. - progress).powi(2);
        let aa = mode.effective_blur();
        let unit = snap.window_height / 2.;
        let (cx, cy) = anim.rect.center();
        let half = (anim.rect.w / 2., anim.rect.h / 2.);
        let alpha = |sdf: f32| (1. - smoothstep(-aa, aa, sdf)) * fade;

        let mut em = Emitter {
            tris,
            clip: snap.pane_rect,
            exclude: None,
            fade: None,
        };
        match mode.preset {
            CursorModeChangePreset::None => {}
            CursorModeChangePreset::Ripple => {
                let r = eased * mode.effective_max_radius() * unit;
                let t = mode.ring_thickness * unit / 2.;
                let levels = [r - t - aa, r - t, r - t + aa, r + t - aa, r + t, r + t + aa];
                bands(
                    &levels,
                    0.,
                    |d| alpha((d - r).abs() - t),
                    |r0, a0, r1, a1| em.annulus(cx, cy, r0, a0, r1, a1),
                );
            }
            CursorModeChangePreset::SonicBoom => {
                let r = eased * mode.effective_max_radius() * unit;
                bands(
                    &[0., r - aa, r, r + aa],
                    0.,
                    |d| alpha(d - r),
                    |r0, a0, r1, a1| em.annulus(cx, cy, r0, a0, r1, a1),
                );
            }
            CursorModeChangePreset::RectangleBoom => {
                let grow = eased * mode.max_size * unit;
                let half = (half.0 + grow, half.1 + grow);
                let core = -half.0.min(half.1);
                bands(&[core, -aa, 0., aa], core, alpha, |d0, a0, d1, a1| {
                    em.rect_frame(cx, cy, half, d0, a0, d1, a1)
                });
            }
            CursorModeChangePreset::RectangleRipple => {
                let grow = eased * mode.max_size * unit;
                let half = (half.0 + grow, half.1 + grow);
                let t = mode.ring_thickness * unit / 2.;
                let levels = [-t - aa, -t, -t + aa, t - aa, t, t + aa];
                bands(
                    &levels,
                    -half.0.min(half.1),
                    |d| alpha(d.abs() - t),
                    |d0, a0, d1, a1| em.rect_frame(cx, cy, half, d0, a0, d1, a1),
                );
            }
        }
        Some(anim.start + Duration::from_secs_f32(duration * (1. - mode.animation_start_offset)))
    }
}

/// Buffers reused from frame to frame
#[derive(Default)]
pub struct Scratch {
    tris: Vec<[Pt; 3]>,
    verts: Vec<Vertex>,
}

impl crate::TermWindow {
    /// Update the animation state of `pos` from the cursor that was just
    /// rendered, and draw any active effect.  This runs after the pane's
    /// lines so the effect sits above cell backgrounds and below text.
    pub fn paint_cursor_animation(
        &mut self,
        pos: &PositionedPane,
        rendered: Option<RenderedCursor>,
        pane_rect: Rect,
        layers: &mut TripleLayerQuadAllocator,
    ) -> anyhow::Result<()> {
        let config = &self.config.cursor_animation;
        if !config.enabled {
            return Ok(());
        }
        let pane_id = pos.pane.pane_id();
        let now = Instant::now();

        let eligible = pos.is_active
            && self.focused.is_some()
            && self.dead_key_status == DeadKeyStatus::None
            && !self.leader_is_active();
        let input = match rendered {
            _ if !eligible => CursorInput::Inactive,
            // Not drawn: hidden, or outside the viewport.  Scrolling the
            // viewport changes the layout key, so it snaps on return.
            None => CursorInput::Hidden,
            Some(cursor) => {
                let mut hasher = DefaultHasher::new();
                for v in [pane_rect.x, pane_rect.y, pane_rect.w, pane_rect.h] {
                    v.to_bits().hash(&mut hasher);
                }
                (
                    self.dimensions.pixel_width,
                    self.dimensions.pixel_height,
                    self.dimensions.dpi,
                    self.render_metrics.cell_size,
                    self.get_viewport(pane_id),
                    pos.pane.is_alt_screen_active(),
                    self.config.generation(),
                )
                    .hash(&mut hasher);
                CursorInput::Visible(CursorVisualSnapshot {
                    rect: cursor.rect,
                    kind: cursor.kind,
                    color: cursor.color,
                    cell_height: self.render_metrics.cell_size.height as f32,
                    pane_rect,
                    window_height: self.dimensions.pixel_height as f32,
                    layout_key: hasher.finish(),
                })
            }
        };

        let mut state = self.pane_state(pane_id);
        let anim = &mut state.cursor_anim;
        anim.mark_painted(self.paint_generation);
        anim.update(input, now, config);
        let cursor_color = match anim.previous {
            Some(snap) if anim.is_animating() => snap.color,
            _ => return Ok(()),
        };

        let mut scratch = self.cursor_anim_scratch.borrow_mut();
        let Scratch { tris, verts } = &mut *scratch;
        verts.clear();
        let offset = (
            self.dimensions.pixel_width as f32 / 2.,
            self.dimensions.pixel_height as f32 / 2.,
        );
        let texture = self
            .render_state
            .as_ref()
            .unwrap()
            .util_sprites
            .filled_box
            .texture_coords();
        let color_or_cursor =
            |c: &Option<config::RgbaColor>| c.map(|c| c.to_linear()).unwrap_or(cursor_color);

        tris.clear();
        let motion_end = anim.render_motion(now, config, tris);
        triangles_to_vertices(
            tris,
            color_or_cursor(&config.motion.color),
            offset,
            texture,
            verts,
        );
        tris.clear();
        let mode_end = anim.render_mode_change(now, config, tris);
        triangles_to_vertices(
            tris,
            color_or_cursor(&config.mode_change.color),
            offset,
            texture,
            verts,
        );
        drop(state);

        if !verts.is_empty() {
            layers.extend_with(0, verts);
        }

        let end = match (motion_end, mode_end) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if let Some(end) = end {
            let fps = config
                .fps
                .unwrap_or(self.config.animation_fps as u16)
                .max(1);
            let frame = Duration::from_secs_f32(1. / fps as f32);
            // Always render the frame at `end`, which clears the effect
            self.update_next_frame_time(Some((now + frame).min(end)));
        }
        Ok(())
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 - e0 <= f32::EPSILON {
        return if x < e0 { 0. } else { 1. };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Call `emit(l0, alpha(l0), l1, alpha(l1))` for each band between the
/// sorted `levels`, ignoring levels below `min`.
fn bands(
    levels: &[f32],
    min: f32,
    alpha: impl Fn(f32) -> f32,
    mut emit: impl FnMut(f32, f32, f32, f32),
) {
    let mut sorted = [0f32; 8];
    let n = levels.len().min(sorted.len());
    for (dst, src) in sorted.iter_mut().zip(levels) {
        *dst = src.max(min);
    }
    let sorted = &mut sorted[..n];
    sorted.sort_by(|a, b| a.total_cmp(b));
    for pair in sorted.windows(2) {
        if pair[1] - pair[0] > 1e-3 {
            emit(pair[0], alpha(pair[0]), pair[1], alpha(pair[1]));
        }
    }
}

/// Moving along the main diagonal (down-right or up-left in window
/// coordinates) connects the top-right and bottom-left corners;
/// otherwise the top-left and bottom-right corners.
fn is_main_diagonal(from: Rect, to: Rect) -> bool {
    (to.x > from.x && to.y > from.y) || (to.x < from.x && to.y < from.y)
}

fn is_straight(from: Rect, to: Rect) -> bool {
    let (fx, fy) = from.center();
    let (tx, ty) = to.center();
    (fx - tx).abs() < 0.5 || (fy - ty).abs() < 0.5
}

/// Rectangle covering both centers, sized like the destination cursor
fn span_rect(c0: (f32, f32), c1: (f32, f32), size: (f32, f32)) -> Poly {
    let x0 = c0.0.min(c1.0) - size.0 / 2.;
    let x1 = c0.0.max(c1.0) + size.0 / 2.;
    let y0 = c0.1.min(c1.1) - size.1 / 2.;
    let y1 = c0.1.max(c1.1) + size.1 / 2.;
    Poly::from_xy(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

/// The front (head) and back (tail) edges of a trail between two cursor
/// positions whose top-left corners are `head` and `tail`.
fn diagonal_edges(from: Rect, to: Rect, head: (f32, f32), tail: (f32, f32)) -> [(f32, f32); 4] {
    let w = to.w;
    if is_main_diagonal(from, to) {
        [
            (head.0, head.1 + to.h),
            (head.0 + w, head.1),
            (tail.0 + w, tail.1),
            (tail.0, tail.1 + from.h),
        ]
    } else {
        [
            (head.0 + w, head.1 + to.h),
            (head.0, head.1),
            (tail.0, tail.1),
            (tail.0 + w, tail.1 + from.h),
        ]
    }
}

/// cursor_tail.glsl: the head and the tail of the trail advance with
/// separate progress curves.  For moves longer than `max_length` the tail
/// is held back until the head has travelled `max_length`; shorter moves
/// have the head arrive immediately while the tail catches up.
fn tail_polygon(
    from: Rect,
    to: Rect,
    progress: f32,
    max_length: f32,
    ease: impl Fn(f32) -> f32,
) -> Poly {
    let (fx, fy) = from.center();
    let (tx, ty) = to.center();
    let length = (tx - fx).hypot(ty - fy);
    let (head, tail) = if length >= max_length {
        (
            ease(progress),
            ease(smoothstep(max_length / length, 1., progress)),
        )
    } else {
        (1., ease(progress))
    };

    if is_straight(from, to) {
        let head = (lerp(fx, tx, head), lerp(fy, ty, head));
        let tail = (lerp(fx, tx, tail), lerp(fy, ty, tail));
        return span_rect(head, tail, (to.w, to.h));
    }
    let head_tl = (lerp(from.x, to.x, head), lerp(from.y, to.y, head));
    let tail_tl = (lerp(from.x, to.x, tail), lerp(from.y, to.y, tail));
    Poly::from_xy(&diagonal_edges(from, to, head_tl, tail_tl))
}

/// cursor_sweep.glsl: a trail attached to the new cursor that starts at
/// `trail_length` of the way back to the old position and shrinks away.
fn sweep_polygon(from: Rect, to: Rect, shrink: f32, trail_length: f32) -> Poly {
    let back = trail_length * (1. - shrink);
    if is_straight(from, to) {
        let (fx, fy) = from.center();
        let (tx, ty) = to.center();
        let tail = (lerp(tx, fx, back), lerp(ty, fy, back));
        return span_rect((tx, ty), tail, (to.w, to.h));
    }
    let [v0, v1, v2_full, v3_full] = diagonal_edges(from, to, (to.x, to.y), (from.x, from.y));
    let v2 = (lerp(v1.0, v2_full.0, back), lerp(v1.1, v2_full.1, back));
    let v3 = (lerp(v0.0, v3_full.0, back), lerp(v0.1, v3_full.1, back));
    Poly::from_xy(&[v0, v1, v2, v3])
}

/// cursor_warp.glsl: each corner travels from the old cursor to the new
/// one on its own schedule; corners facing the direction of travel
/// arrive first, stretching the cursor into a quad.
#[allow(clippy::too_many_arguments)]
fn warp_polygon(
    from: Rect,
    to: Rect,
    elapsed: f32,
    duration: f32,
    trail_size: f32,
    thickness: f32,
    thickness_x: f32,
    ease: impl Fn(f32) -> f32,
) -> Poly {
    let lead = duration * (1. - trail_size);
    let trail = duration;
    let side = (lead + trail) / 2.;
    let dur_for = |dot: f32| {
        if dot >= 0.5 {
            lead
        } else if dot >= -0.5 {
            side
        } else {
            trail
        }
    };
    let (fx, fy) = from.center();
    let (tx, ty) = to.center();
    let sign = |v: f32| {
        if v > 0. {
            1.
        } else if v < 0. {
            -1.
        } else {
            0.
        }
    };
    let (sx, sy) = (sign(tx - fx), sign(ty - fy));
    // Corner directions: TL, TR, BR, BL
    let dirs = [(-1f32, -1f32), (1., -1.), (1., 1.), (-1., 1.)];
    let dots = dirs.map(|(dx, dy)| dx * sx + dy * sy);
    let right_rail = dur_for((dots[1] + dots[2]) / 2.);
    let left_rail = dur_for((dots[0] + dots[3]) / 2.);
    let mut durations = dots.map(dur_for);
    if sx > 0. {
        durations[1] = right_rail;
        durations[2] = right_rail;
    } else if sx < 0. {
        durations[0] = left_rail;
        durations[3] = left_rail;
    }

    let corners = |r: Rect| {
        let (cx, cy) = r.center();
        let hw = r.w / 2. * thickness_x;
        let hh = r.h / 2. * thickness;
        dirs.map(|(dx, dy)| (cx + dx * hw, cy + dy * hh))
    };
    let (src, dst) = (corners(from), corners(to));
    let mut pts = [(0f32, 0f32); 4];
    for i in 0..4 {
        let p = ease((elapsed / durations[i].max(1e-6)).clamp(0., 1.));
        pts[i] = (lerp(src[i].0, dst[i].0, p), lerp(src[i].1, dst[i].1, p));
    }
    Poly::from_xy(&pts)
}

/// A polygon vertex with alpha
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
    pub a: f32,
}

impl Pt {
    fn lerp(self, other: Pt, t: f32) -> Pt {
        Pt {
            x: lerp(self.x, other.x, t),
            y: lerp(self.y, other.y, t),
            a: lerp(self.a, other.a, t),
        }
    }
    fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.a.is_finite()
    }
}

/// Clipping a convex polygon against a half plane adds at most one
/// vertex; our inputs have at most 4 and we clip at most 8 times.
const MAX_PTS: usize = 16;

/// A small convex polygon that lives on the stack
#[derive(Clone, Copy, Debug)]
pub struct Poly {
    pts: [Pt; MAX_PTS],
    n: usize,
}

impl Poly {
    fn new(src: &[Pt]) -> Self {
        let mut p = Self {
            pts: [Pt::default(); MAX_PTS],
            n: 0,
        };
        for &pt in src {
            p.push(pt);
        }
        p
    }

    fn from_xy(src: &[(f32, f32)]) -> Self {
        let mut p = Self::new(&[]);
        for &(x, y) in src {
            p.push(Pt { x, y, a: 1. });
        }
        p
    }

    fn push(&mut self, pt: Pt) {
        if self.n < MAX_PTS {
            self.pts[self.n] = pt;
            self.n += 1;
        }
    }

    pub fn as_slice(&self) -> &[Pt] {
        &self.pts[..self.n]
    }

    /// Keep the part of the polygon where `nx * x + ny * y >= c`
    fn clip(&self, nx: f32, ny: f32, c: f32) -> Self {
        let mut out = Self::new(&[]);
        let side = |p: &Pt| nx * p.x + ny * p.y - c;
        for i in 0..self.n {
            let cur = self.pts[i];
            let next = self.pts[(i + 1) % self.n];
            let (dc, dn) = (side(&cur), side(&next));
            if dc >= 0. {
                out.push(cur);
            }
            if (dc >= 0.) != (dn >= 0.) {
                out.push(cur.lerp(next, dc / (dc - dn)));
            }
        }
        out
    }

    fn signed_area(&self) -> f32 {
        let mut area = 0.;
        for i in 0..self.n {
            let (a, b) = (self.pts[i], self.pts[(i + 1) % self.n]);
            area += a.x * b.y - b.x * a.y;
        }
        area / 2.
    }
}

/// Alpha fade along the direction of travel (Warp `fade`)
#[derive(Clone, Copy, Debug)]
struct Fade {
    ox: f32,
    oy: f32,
    dx: f32,
    dy: f32,
    exponent: f32,
}

const FADE_BANDS: usize = 12;
const CIRCLE_SEGMENTS: usize = 48;

/// Clips convex polygons and collects them as triangles
struct Emitter<'a> {
    tris: &'a mut Vec<[Pt; 3]>,
    clip: Rect,
    exclude: Option<Rect>,
    fade: Option<Fade>,
}

impl Emitter<'_> {
    fn convex(&mut self, pts: &[Pt]) {
        if pts.len() < 3 || !pts.iter().all(Pt::is_finite) {
            return;
        }
        let c = self.clip;
        let p = Poly::new(pts)
            .clip(1., 0., c.x)
            .clip(-1., 0., -c.right())
            .clip(0., 1., c.y)
            .clip(0., -1., -c.bottom());
        match self.exclude {
            None => self.faded(p),
            Some(r) => {
                // The polygon minus the rectangle, as four convex pieces
                self.faded(p.clip(-1., 0., -r.x));
                self.faded(p.clip(1., 0., r.right()));
                let mid = p.clip(1., 0., r.x).clip(-1., 0., -r.right());
                self.faded(mid.clip(0., -1., -r.y));
                self.faded(mid.clip(0., 1., r.bottom()));
            }
        }
    }

    fn faded(&mut self, p: Poly) {
        if p.n < 3 {
            return;
        }
        let f = match self.fade {
            Some(f) => f,
            None => return self.fan(&p),
        };
        let len2 = f.dx * f.dx + f.dy * f.dy;
        if len2 < 1e-6 {
            return self.fan(&p);
        }
        let base = f.dx * f.ox + f.dy * f.oy;
        let level = |k: usize| base + len2 * k as f32 / FADE_BANDS as f32;
        for k in 0..FADE_BANDS {
            let mut band = p;
            if k > 0 {
                band = band.clip(f.dx, f.dy, level(k));
            }
            if k + 1 < FADE_BANDS {
                band = band.clip(-f.dx, -f.dy, -level(k + 1));
            }
            for pt in &mut band.pts[..band.n] {
                let t = ((f.dx * pt.x + f.dy * pt.y - base) / len2).clamp(0., 1.);
                pt.a *= t.powf(f.exponent);
            }
            self.fan(&band);
        }
    }

    fn fan(&mut self, p: &Poly) {
        for i in 1..p.n.saturating_sub(1) {
            self.tris.push([p.pts[0], p.pts[i], p.pts[i + 1]]);
        }
    }

    /// Fill a simple polygon with at most one reflex vertex (the Warp
    /// quad can be concave) by fanning out from that vertex.
    fn simple(&mut self, pts: &[Pt], orient: f32) {
        let n = pts.len();
        let reflex = (0..n).find(|&i| {
            let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
            cross * orient < -1e-3
        });
        match reflex {
            None => self.convex(pts),
            Some(k) => {
                for i in 1..n - 1 {
                    self.convex(&[pts[k], pts[(k + i) % n], pts[(k + i + 1) % n]]);
                }
            }
        }
    }

    /// Fill a polygon and surround it with a `blur` pixel wide
    /// strip whose alpha falls off to zero.
    fn feathered(&mut self, pts: &[Pt], blur: f32) {
        let poly = Poly::new(pts);
        let area = poly.signed_area();
        self.simple(pts, area.signum());
        if !(blur > 0.01 && area.abs() > 1e-3) {
            return;
        }
        let orient = area.signum();

        // Outward normals of the non-degenerate edges
        let mut edges = [(Pt::default(), Pt::default(), 0f32, 0f32); MAX_PTS];
        let mut n = 0;
        for i in 0..poly.n {
            let (a, b) = (poly.pts[i], poly.pts[(i + 1) % poly.n]);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len = dx.hypot(dy);
            if len > 1e-3 {
                edges[n] = (a, b, orient * dy / len, -orient * dx / len);
                n += 1;
            }
        }
        let out = |p: Pt, nx: f32, ny: f32| Pt {
            x: p.x + nx * blur,
            y: p.y + ny * blur,
            a: 0.,
        };
        for i in 0..n {
            let (a, b, nx, ny) = edges[i];
            self.convex(&[a, b, out(b, nx, ny), out(a, nx, ny)]);
            let (_, _, mx, my) = edges[(i + 1) % n];
            self.convex(&[b, out(b, nx, ny), out(b, mx, my)]);
        }
    }

    fn annulus(&mut self, cx: f32, cy: f32, r0: f32, a0: f32, r1: f32, a1: f32) {
        let step = std::f32::consts::TAU / CIRCLE_SEGMENTS as f32;
        let at = |i: usize, r: f32, a: f32| {
            let (s, c) = (i as f32 * step).sin_cos();
            Pt {
                x: cx + c * r,
                y: cy + s * r,
                a,
            }
        };
        for i in 0..CIRCLE_SEGMENTS {
            let j = i + 1;
            self.convex(&[at(i, r0, a0), at(i, r1, a1), at(j, r1, a1), at(j, r0, a0)]);
        }
    }

    /// The region between the rectangles centered on (cx, cy) whose
    /// half sizes are `half` grown by `d0` and `d1`.
    #[allow(clippy::too_many_arguments)]
    fn rect_frame(
        &mut self,
        cx: f32,
        cy: f32,
        half: (f32, f32),
        d0: f32,
        a0: f32,
        d1: f32,
        a1: f32,
    ) {
        let corners = |d: f32, a: f32| {
            let (hx, hy) = ((half.0 + d).max(0.), (half.1 + d).max(0.));
            [
                Pt {
                    x: cx - hx,
                    y: cy - hy,
                    a,
                },
                Pt {
                    x: cx + hx,
                    y: cy - hy,
                    a,
                },
                Pt {
                    x: cx + hx,
                    y: cy + hy,
                    a,
                },
                Pt {
                    x: cx - hx,
                    y: cy + hy,
                    a,
                },
            ]
        };
        let (inner, outer) = (corners(d0, a0), corners(d1, a1));
        for i in 0..4 {
            let j = (i + 1) % 4;
            self.convex(&[outer[i], outer[j], inner[j], inner[i]]);
        }
    }
}

/// Convert triangles into renderer quads (the last vertex of each quad
/// is repeated, so the second triangle of the quad is degenerate).
/// `offset` is subtracted from positions: the renderer puts the origin
/// at the center of the window.
pub fn triangles_to_vertices(
    tris: &[[Pt; 3]],
    color: LinearRgba,
    offset: (f32, f32),
    texture: TextureRect,
    out: &mut Vec<Vertex>,
) {
    let (r, g, b, a) = color.tuple();
    let tex = [texture.min_x(), texture.min_y()];
    for tri in tris {
        if !tri.iter().all(Pt::is_finite) {
            continue;
        }
        for p in [tri[0], tri[1], tri[2], tri[2]] {
            let color = [r, g, b, a * p.a.clamp(0., 1.)];
            out.push(Vertex {
                position: [p.x - offset.0, p.y - offset.1],
                tex,
                fg_color: color,
                alt_color: color,
                hsv: [1., 1., 1.],
                has_color: IS_SOLID_COLOR,
                mix_value: 0.,
            });
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use config::CursorMotionPreset;

    fn cfg(motion: CursorMotionPreset, mode: CursorModeChangePreset) -> CursorAnimation {
        let mut c = CursorAnimation {
            enabled: true,
            ..Default::default()
        };
        c.motion.preset = motion;
        c.mode_change.preset = mode;
        c
    }

    fn snap(col: f32, row: f32, kind: CursorKind) -> CursorVisualSnapshot {
        let (cw, ch) = (10., 20.);
        let rect = match kind {
            CursorKind::Block => Rect::new(col * cw, row * ch, cw, ch),
            CursorKind::Bar => Rect::new(col * cw, row * ch, 2., ch),
            CursorKind::Underline => Rect::new(col * cw, row * ch + ch - 2., cw, 2.),
        };
        CursorVisualSnapshot {
            rect,
            kind,
            color: LinearRgba::with_components(1., 1., 1., 1.),
            cell_height: ch,
            pane_rect: Rect::new(0., 0., 800., 600.),
            window_height: 600.,
            layout_key: 1,
        }
    }

    fn tail() -> CursorAnimation {
        cfg(CursorMotionPreset::Tail, CursorModeChangePreset::Ripple)
    }

    #[test]
    fn threshold() {
        let c = tail();
        let now = Instant::now();
        let mut st = CursorAnimState::default();
        st.update(
            CursorInput::Visible(snap(0., 0., CursorKind::Block)),
            now,
            &c,
        );
        // One cell right: 10px < 1.5 * 20px
        st.update(
            CursorInput::Visible(snap(1., 0., CursorKind::Block)),
            now,
            &c,
        );
        assert!(!st.is_animating());
        // Far away
        st.update(
            CursorInput::Visible(snap(40., 10., CursorKind::Block)),
            now,
            &c,
        );
        assert!(st.motion.is_some());
        // A following small move cancels the trail
        st.update(
            CursorInput::Visible(snap(41., 10., CursorKind::Block)),
            now,
            &c,
        );
        assert!(st.motion.is_none());
    }

    #[test]
    fn replaced_by_new_move() {
        let c = tail();
        let t0 = Instant::now();
        let mut st = CursorAnimState::default();
        st.update(
            CursorInput::Visible(snap(0., 0., CursorKind::Block)),
            t0,
            &c,
        );
        st.update(
            CursorInput::Visible(snap(40., 0., CursorKind::Block)),
            t0,
            &c,
        );
        let t1 = t0 + Duration::from_millis(30);
        st.update(
            CursorInput::Visible(snap(0., 20., CursorKind::Block)),
            t1,
            &c,
        );
        let m = st.motion.unwrap();
        assert_eq!(m.from, snap(40., 0., CursorKind::Block).rect);
        assert_eq!(m.start, t1);
    }

    #[test]
    fn completion_stops_frames() {
        let c = tail();
        let t0 = Instant::now();
        let mut st = CursorAnimState::default();
        st.update(
            CursorInput::Visible(snap(0., 0., CursorKind::Block)),
            t0,
            &c,
        );
        st.update(
            CursorInput::Visible(snap(50., 20., CursorKind::Block)),
            t0,
            &c,
        );
        let mut tris = vec![];
        let end = st
            .render_motion(t0 + Duration::from_millis(40), &c, &mut tris)
            .unwrap();
        assert_eq!(end, t0 + Duration::from_millis(90));
        assert!(!tris.is_empty());
        tris.clear();
        assert!(st
            .render_motion(t0 + Duration::from_millis(90), &c, &mut tris)
            .is_none());
        assert!(tris.is_empty());
        assert!(!st.is_animating());
    }

    #[test]
    fn resets() {
        let c = tail();
        let now = Instant::now();
        let mut st = CursorAnimState::default();
        st.update(
            CursorInput::Visible(snap(0., 0., CursorKind::Block)),
            now,
            &c,
        );

        // Layout (resize, scroll, focus, config...) changes snap
        let mut moved = snap(40., 10., CursorKind::Bar);
        moved.layout_key = 2;
        st.update(CursorInput::Visible(moved), now, &c);
        assert!(!st.is_animating());

        // Inactive (unfocused, other pane...) resets; the next position
        // is a fresh start
        st.update(CursorInput::Inactive, now, &c);
        assert!(st.previous.is_none());
        let mut back = snap(0., 0., CursorKind::Block);
        back.layout_key = 2;
        st.update(CursorInput::Visible(back), now, &c);
        assert!(!st.is_animating());
    }

    #[test]
    fn hidden_keeps_last_position() {
        let c = tail();
        let now = Instant::now();
        let mut st = CursorAnimState::default();
        st.update(
            CursorInput::Visible(snap(0., 0., CursorKind::Block)),
            now,
            &c,
        );
        // Applications hide the cursor while redrawing, then show it
        // at the new position
        st.update(CursorInput::Hidden, now, &c);
        assert!(!st.is_animating());
        st.update(
            CursorInput::Visible(snap(40., 20., CursorKind::Block)),
            now,
            &c,
        );
        assert!(st.is_animating());
        // Hidden mid-animation: nothing drawn, resumes when shown
        st.update(CursorInput::Hidden, now, &c);
        assert!(!st.is_animating());
        st.update(
            CursorInput::Visible(snap(40., 20., CursorKind::Block)),
            now,
            &c,
        );
        assert!(st.is_animating());
    }

    #[test]
    fn skipped_paint_resets() {
        let c = tail();
        let now = Instant::now();
        let mut st = CursorAnimState::default();
        st.mark_painted(1);
        st.update(
            CursorInput::Visible(snap(0., 0., CursorKind::Block)),
            now,
            &c,
        );
        // Painted twice in the same frame, then in the next frame
        st.mark_painted(1);
        st.mark_painted(2);
        assert!(st.previous.is_some());
        // Tab was hidden for frames 3..=9
        st.mark_painted(10);
        st.update(
            CursorInput::Visible(snap(40., 20., CursorKind::Block)),
            now,
            &c,
        );
        assert!(!st.is_animating());
    }

    #[test]
    fn mode_change_detection() {
        let c = tail();
        let now = Instant::now();
        let mut st = CursorAnimState::default();
        st.update(
            CursorInput::Visible(snap(3., 3., CursorKind::Block)),
            now,
            &c,
        );
        // Redraw with identical state (eg: blink toggle) does nothing
        st.update(
            CursorInput::Visible(snap(3., 3., CursorKind::Block)),
            now,
            &c,
        );
        assert!(st.mode_change.is_none());
        st.update(CursorInput::Visible(snap(3., 3., CursorKind::Bar)), now, &c);
        assert!(st.mode_change.is_some());
        // Shape change in place must not produce a trail
        assert!(st.motion.is_none());

        let mut st = CursorAnimState::default();
        st.update(
            CursorInput::Visible(snap(3., 3., CursorKind::Block)),
            now,
            &c,
        );
        st.update(
            CursorInput::Visible(snap(3., 3., CursorKind::Underline)),
            now,
            &c,
        );
        assert!(st.mode_change.is_some());

        // Blinking and steady variants are the same kind
        assert_eq!(
            CursorKind::from_shape(CursorShape::BlinkingBlock),
            CursorKind::from_shape(CursorShape::SteadyBlock)
        );
        assert_eq!(
            CursorKind::from_shape(CursorShape::Default),
            CursorKind::Block
        );

        // Focus loss resets rather than triggering on the way back
        let mut st = CursorAnimState::default();
        st.update(CursorInput::Visible(snap(3., 3., CursorKind::Bar)), now, &c);
        st.update(CursorInput::Inactive, now, &c);
        st.update(
            CursorInput::Visible(snap(3., 3., CursorKind::Block)),
            now,
            &c,
        );
        assert!(st.mode_change.is_none());
    }

    #[test]
    fn mode_change_disabled() {
        let c = cfg(CursorMotionPreset::None, CursorModeChangePreset::None);
        let now = Instant::now();
        let mut st = CursorAnimState::default();
        st.update(
            CursorInput::Visible(snap(0., 0., CursorKind::Block)),
            now,
            &c,
        );
        st.update(
            CursorInput::Visible(snap(30., 10., CursorKind::Bar)),
            now,
            &c,
        );
        assert!(!st.is_animating());
    }

    fn bounds(tris: &[[Pt; 3]]) -> Rect {
        let pts = tris.iter().flatten();
        let x0 = pts.clone().map(|p| p.x).fold(f32::MAX, f32::min);
        let x1 = pts.clone().map(|p| p.x).fold(f32::MIN, f32::max);
        let y0 = pts.clone().map(|p| p.y).fold(f32::MAX, f32::min);
        let y1 = pts.map(|p| p.y).fold(f32::MIN, f32::max);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    fn inside(r: &Rect, p: &Pt) -> bool {
        p.x >= r.x - 1e-3
            && p.x <= r.right() + 1e-3
            && p.y >= r.y - 1e-3
            && p.y <= r.bottom() + 1e-3
    }

    fn tri_area(t: &[Pt; 3]) -> f32 {
        ((t[1].x - t[0].x) * (t[2].y - t[0].y) - (t[2].x - t[0].x) * (t[1].y - t[0].y)).abs() / 2.
    }

    /// Sample the midpoint of the move at 40% progress for each direction
    fn render_tail(from: (f32, f32), to: (f32, f32)) -> Vec<[Pt; 3]> {
        let c = tail();
        let t0 = Instant::now();
        let mut st = CursorAnimState::default();
        st.update(
            CursorInput::Visible(snap(from.0, from.1, CursorKind::Block)),
            t0,
            &c,
        );
        st.update(
            CursorInput::Visible(snap(to.0, to.1, CursorKind::Block)),
            t0,
            &c,
        );
        let mut tris = vec![];
        st.render_motion(t0 + Duration::from_millis(36), &c, &mut tris);
        tris
    }

    #[test]
    fn tail_directions() {
        // horizontal, vertical, both diagonals, both ways
        for (from, to) in [
            ((0., 5.), (60., 5.)),
            ((60., 5.), (0., 5.)),
            ((5., 0.), (5., 25.)),
            ((5., 25.), (5., 0.)),
            ((0., 0.), (60., 25.)),
            ((60., 25.), (0., 0.)),
            ((0., 25.), (60., 0.)),
            ((60., 0.), (0., 25.)),
        ] {
            let tris = render_tail(from, to);
            assert!(!tris.is_empty(), "{:?} -> {:?}", from, to);
            let area: f32 = tris.iter().map(tri_area).sum();
            assert!(area > 100., "{:?} -> {:?} area {}", from, to, area);
            let dest = snap(to.0, to.1, CursorKind::Block).rect;
            let pane = snap(0., 0., CursorKind::Block).pane_rect;
            for t in &tris {
                assert!(t.iter().all(Pt::is_finite));
                assert!(t.iter().all(|p| inside(&pane, p)));
                // Nothing may be drawn inside the destination cursor
                let (cx, cy) = (
                    (t[0].x + t[1].x + t[2].x) / 3.,
                    (t[0].y + t[1].y + t[2].y) / 3.,
                );
                let strictly_inside = cx > dest.x + 1e-3
                    && cx < dest.right() - 1e-3
                    && cy > dest.y + 1e-3
                    && cy < dest.bottom() - 1e-3;
                assert!(
                    !strictly_inside || tri_area(t) < 1e-3,
                    "{:?} -> {:?} drew over the cursor",
                    from,
                    to
                );
            }
            // The trail lies between the two positions
            let b = bounds(&tris);
            let from_rect = snap(from.0, from.1, CursorKind::Block).rect;
            let lo = from_rect.x.min(dest.x) - 3.;
            let hi = from_rect.right().max(dest.right()) + 3.;
            assert!(
                b.x >= lo && b.right() <= hi,
                "{:?} -> {:?} {:?}",
                from,
                to,
                b
            );
        }
    }

    #[test]
    fn clipped_to_pane() {
        let mut em_tris = vec![];
        let pane = Rect::new(100., 100., 50., 50.);
        let mut em = Emitter {
            tris: &mut em_tris,
            clip: pane,
            exclude: None,
            fade: None,
        };
        em.feathered(
            Poly::from_xy(&[(0., 0.), (300., 0.), (300., 300.), (0., 300.)]).as_slice(),
            4.,
        );
        em.annulus(125., 125., 0., 1., 500., 0.);
        assert!(!em_tris.is_empty());
        for t in &em_tris {
            assert!(t.iter().all(|p| inside(&pane, p)));
        }
    }

    #[test]
    fn exclusion_leaves_hole() {
        let mut tris = vec![];
        let hole = Rect::new(10., 10., 10., 10.);
        let mut em = Emitter {
            tris: &mut tris,
            clip: Rect::new(0., 0., 30., 30.),
            exclude: Some(hole),
            fade: None,
        };
        em.convex(Poly::from_xy(&[(0., 0.), (30., 0.), (30., 30.), (0., 30.)]).as_slice());
        let area: f32 = tris.iter().map(tri_area).sum();
        assert!((area - 800.).abs() < 1e-2, "area {}", area);
    }

    #[test]
    fn concave_quad() {
        // (10,10) is a reflex vertex; a fan from (20,0) would
        // cover 250 instead of the true area of 150
        let mut tris = vec![];
        let mut em = Emitter {
            tris: &mut tris,
            clip: Rect::new(-100., -100., 200., 200.),
            exclude: None,
            fade: None,
        };
        em.feathered(
            Poly::from_xy(&[(20., 0.), (10., 10.), (10., 20.), (0., 0.)])
                .as_slice()
                .iter()
                .rev()
                .copied()
                .collect::<Vec<_>>()
                .as_slice(),
            0.,
        );
        let area: f32 = tris.iter().map(tri_area).sum();
        assert!((area - 150.).abs() < 1e-2, "area {}", area);
    }

    #[test]
    fn degenerate_input() {
        let mut tris = vec![];
        let mut em = Emitter {
            tris: &mut tris,
            clip: Rect::new(0., 0., 0., 0.),
            exclude: Some(Rect::default()),
            fade: None,
        };
        em.feathered(
            Poly::from_xy(&[(1., 1.), (1., 1.), (1., 1.)]).as_slice(),
            2.,
        );
        em.feathered(
            Poly::from_xy(&[(f32::NAN, 1.), (5., 1.), (5., 5.)]).as_slice(),
            2.,
        );
        em.annulus(0., 0., 0., 1., 0., 1.);
        em.rect_frame(0., 0., (0., 0.), -1., 1., 1., 0.);
        assert!(tris.iter().all(|t| t.iter().all(Pt::is_finite)));

        let mut st = CursorAnimState::default();
        let mut bad = snap(0., 0., CursorKind::Block);
        bad.rect.w = f32::NAN;
        st.update(CursorInput::Visible(bad), Instant::now(), &tail());
        assert!(st.previous.is_none());
    }

    #[test]
    fn all_presets_render() {
        for motion in [
            CursorMotionPreset::Tail,
            CursorMotionPreset::Warp,
            CursorMotionPreset::Sweep,
        ] {
            for mode in [
                CursorModeChangePreset::Ripple,
                CursorModeChangePreset::SonicBoom,
                CursorModeChangePreset::RectangleBoom,
                CursorModeChangePreset::RectangleRipple,
            ] {
                let mut c = cfg(motion, mode);
                c.motion.warp.fade = true;
                let t0 = Instant::now();
                let mut st = CursorAnimState::default();
                st.update(
                    CursorInput::Visible(snap(2., 2., CursorKind::Block)),
                    t0,
                    &c,
                );
                st.update(
                    CursorInput::Visible(snap(50., 20., CursorKind::Bar)),
                    t0,
                    &c,
                );
                let now = t0 + Duration::from_millis(50);
                let mut tris = vec![];
                assert!(st.render_motion(now, &c, &mut tris).is_some());
                assert!(!tris.is_empty(), "{:?}", motion);
                let n = tris.len();
                assert!(st.render_mode_change(now, &c, &mut tris).is_some());
                assert!(tris.len() > n, "{:?}", mode);

                let mut verts = vec![];
                triangles_to_vertices(
                    &tris,
                    LinearRgba::with_components(1., 0., 0., 1.),
                    (400., 300.),
                    TextureRect::default(),
                    &mut verts,
                );
                assert_eq!(verts.len() % 4, 0);
                for v in &verts {
                    assert!(v.position.iter().chain(&v.fg_color).all(|f| f.is_finite()));
                    assert!((0. ..=1.).contains(&v.fg_color[3]));
                }
            }
        }
    }

    #[test]
    fn tail_head_leads_tail() {
        // Long move: the tail end lags behind the head
        let from = Rect::new(0., 0., 10., 20.);
        let to = Rect::new(500., 0., 10., 20.);
        let ease = |x: f32| config::CursorEasing::EaseOutCirc.evaluate(x);
        let p = tail_polygon(from, to, 0.3, 60., ease);
        let b = bounds(&[
            [p.pts[0], p.pts[1], p.pts[2]],
            [p.pts[0], p.pts[2], p.pts[3]],
        ]);
        assert!(b.w > 20. && b.right() < 510.);
        // Finished: collapses onto the destination
        let p = tail_polygon(from, to, 1., 60., ease);
        let b = bounds(&[
            [p.pts[0], p.pts[1], p.pts[2]],
            [p.pts[0], p.pts[2], p.pts[3]],
        ]);
        assert!((b.x - 500.).abs() < 1e-3 && (b.w - 10.).abs() < 1e-3);
    }
}
