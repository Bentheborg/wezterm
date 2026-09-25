//! Low-overhead render-latency counters (Phase 1 profiling).
//!
//! Enabled only when `WEZTERM_RENDER_STATS=1` is set at startup.
//! Everything is plain integer accumulation; a report block is logged
//! at most once per second per window, from the start of the next paint.
//! No `metrics::histogram!` calls: those take a global mutex per call.
use mux::pane::PaneId;
use std::collections::HashMap;
use std::fmt::Write;
use std::time::{Duration, Instant};
use termwiz::surface::SequenceNo;
use window::paintstats::PaintStatsSnapshot;

const FRAME_RING: usize = 1024;

/// Per-pane counters for the pane rendering loop
#[derive(Default, Debug, Clone, Copy)]
pub struct PanePerf {
    pub paints: u64,
    pub rows: u64,
    /// Rows whose seqno did not advance since the previous paint of this pane
    pub clean_rows: u64,
    pub quad_hits: u64,
    pub quads_copied_on_hit: u64,
    pub hit_ns: u64,
    /// Quad cache missed but the line-to-element shape cache hit:
    /// same content, different position/cursor/selection/hover
    pub miss_moved_or_context: u64,
    /// Both quad and element-shape caches missed: content changed
    pub miss_content: u64,
    /// Quad cache entry existed but was expired or hover-invalidated
    pub miss_expired_or_hover: u64,
    pub miss_ns: u64,
    pub quads_built: u64,
    /// Duration of Pane::with_lines_mut; for local panes this is
    /// the time the terminal lock is held by the renderer
    pub lock_held_ns: u64,
    pub harfbuzz_calls: u64,
    pub harfbuzz_ns: u64,
}

pub struct RenderPerfStats {
    pub enabled: bool,
    last_report: Instant,

    frame_ms: [f32; FRAME_RING],
    frame_len: usize,
    frames: u64,
    frame_ns: u64,
    paint_pass_runs: u64,
    map_ns: u64,
    unmap_ns: u64,
    call_draw_ns: u64,
    present_ns: u64,
    quads: u64,
    quads_max: u64,

    panes: HashMap<PaneId, PanePerf>,
    /// Highest line seqno seen in the previous paint of each pane;
    /// persists across report windows
    prev_max_seqno: HashMap<PaneId, SequenceNo>,
    cur_max_seqno: SequenceNo,

    /// The pane currently being painted; shaping outside of a pane
    /// (tab bar etc) is attributed to `ui_*`
    current_pane: Option<PaneId>,
    ui_harfbuzz_calls: u64,
    ui_harfbuzz_ns: u64,

    /// Set by render_screen_line when the element-shape cache hit
    pub line_shape_cache_hit: bool,
}

impl Default for RenderPerfStats {
    fn default() -> Self {
        Self {
            enabled: window::paintstats::render_stats_enabled(),
            last_report: Instant::now(),
            frame_ms: [0.; FRAME_RING],
            frame_len: 0,
            frames: 0,
            frame_ns: 0,
            paint_pass_runs: 0,
            map_ns: 0,
            unmap_ns: 0,
            call_draw_ns: 0,
            present_ns: 0,
            quads: 0,
            quads_max: 0,
            panes: HashMap::new(),
            prev_max_seqno: HashMap::new(),
            cur_max_seqno: 0,
            current_pane: None,
            ui_harfbuzz_calls: 0,
            ui_harfbuzz_ns: 0,
            line_shape_cache_hit: false,
        }
    }
}

pub fn ns(d: Duration) -> u64 {
    d.as_nanos() as u64
}

impl RenderPerfStats {
    pub fn pane(&mut self) -> Option<&mut PanePerf> {
        let id = self.current_pane?;
        Some(self.panes.entry(id).or_default())
    }

    /// Called before paint_pane's line loop.
    /// Returns the seqno threshold used to classify clean rows
    /// (None on the first paint of a pane).
    pub fn begin_pane(&mut self, pane_id: PaneId) -> Option<SequenceNo> {
        self.current_pane = Some(pane_id);
        self.cur_max_seqno = 0;
        self.panes.entry(pane_id).or_default().paints += 1;
        self.prev_max_seqno.get(&pane_id).copied()
    }

    pub fn note_line_seqno(&mut self, seqno: SequenceNo) {
        self.cur_max_seqno = self.cur_max_seqno.max(seqno);
    }

    pub fn end_pane(&mut self, pane_id: PaneId, lock_held: Duration) {
        self.prev_max_seqno.insert(pane_id, self.cur_max_seqno);
        if let Some(p) = self.pane() {
            p.lock_held_ns += ns(lock_held);
        }
        self.current_pane = None;
    }

    pub fn record_harfbuzz(&mut self, elapsed: Duration) {
        match self.pane() {
            Some(p) => {
                p.harfbuzz_calls += 1;
                p.harfbuzz_ns += ns(elapsed);
            }
            None => {
                self.ui_harfbuzz_calls += 1;
                self.ui_harfbuzz_ns += ns(elapsed);
            }
        }
    }

    pub fn record_paint_pass(&mut self) {
        self.paint_pass_runs += 1;
    }
    pub fn record_map(&mut self, d: Duration) {
        self.map_ns += ns(d);
    }
    pub fn record_unmap(&mut self, d: Duration) {
        self.unmap_ns += ns(d);
    }
    pub fn record_call_draw(&mut self, d: Duration) {
        self.call_draw_ns += ns(d);
    }
    pub fn record_present(&mut self, d: Duration) {
        self.present_ns += ns(d);
    }

    pub fn record_frame(&mut self, frame: Duration, quads: u64) {
        self.frames += 1;
        self.frame_ns += ns(frame);
        if self.frame_len < FRAME_RING {
            self.frame_ms[self.frame_len] = frame.as_secs_f32() * 1000.;
            self.frame_len += 1;
        }
        self.quads += quads;
        self.quads_max = self.quads_max.max(quads);
    }

    pub fn report_due(&self) -> bool {
        self.last_report.elapsed() >= Duration::from_secs(1)
    }

    /// Logs one report block if at least a second has passed since the
    /// last one, then resets the per-window counters.
    pub fn maybe_report(
        &mut self,
        cols: usize,
        rows: usize,
        max_fps: u64,
        paint: Option<PaintStatsSnapshot>,
        take_parser: impl Fn(PaneId) -> mux::parserperf::ParserPerf,
    ) {
        let elapsed = self.last_report.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }
        let secs = elapsed.as_secs_f64();
        let frames = self.frames;
        let rate = |n: u64| n as f64 / secs;
        let ms = |n: u64| n as f64 / 1e6;
        let per_frame = |n: u64| {
            if frames == 0 {
                0.
            } else {
                n as f64 / frames as f64
            }
        };

        let ring = &mut self.frame_ms[..self.frame_len];
        ring.sort_by(|a, b| a.total_cmp(b));
        let pct = |p: f64| -> f32 {
            if ring.is_empty() {
                0.
            } else {
                ring[((ring.len() - 1) as f64 * p).round() as usize]
            }
        };

        let mut out = String::new();
        let _ = writeln!(
            out,
            "render-stats window={cols}x{rows} max_fps={max_fps} interval={secs:.2}s"
        );
        if let Some(p) = paint {
            let avg = |sum: u64| {
                if p.throttle_count == 0 {
                    0.
                } else {
                    sum as f64 / 1e6 / p.throttle_count as f64
                }
            };
            let _ = writeln!(
                out,
                "  wm_paint/s received={:.1} throttled={:.1} dispatched={:.1} | \
                 throttle interval={:.2}ms requested={:.2}ms actual avg={:.2}ms max={:.2}ms",
                rate(p.wm_paint_received),
                rate(p.wm_paint_throttled),
                rate(p.wm_paint_dispatched),
                avg(p.throttle_interval_ns),
                avg(p.throttle_requested_ns),
                avg(p.throttle_actual_ns),
                ms(p.throttle_actual_max_ns),
            );
        }
        let _ = writeln!(
            out,
            "  paint_impl/s={:.1} paint_pass/frame={:.2} | \
             frame ms avg={:.2} p50={:.2} p95={:.2} p99={:.2} max={:.2}",
            rate(frames),
            per_frame(self.paint_pass_runs),
            ms(per_frame(self.frame_ns) as u64),
            pct(0.50),
            pct(0.95),
            pct(0.99),
            pct(1.0),
        );
        let vertex_bytes = 4 * std::mem::size_of::<crate::quad::Vertex>() as u64;
        let _ = writeln!(
            out,
            "  ms/frame map={:.3} unmap={:.3} call_draw={:.3} present={:.3} | \
             quads/frame avg={:.0} max={} vertex_bytes/frame avg={:.0}",
            per_frame(self.map_ns) / 1e6,
            per_frame(self.unmap_ns) / 1e6,
            per_frame(self.call_draw_ns) / 1e6,
            per_frame(self.present_ns) / 1e6,
            per_frame(self.quads),
            self.quads_max,
            per_frame(self.quads * vertex_bytes),
        );
        if self.ui_harfbuzz_calls > 0 {
            let _ = writeln!(
                out,
                "  ui harfbuzz calls={} ms={:.3}",
                self.ui_harfbuzz_calls,
                ms(self.ui_harfbuzz_ns)
            );
        }

        let mut pane_ids: Vec<PaneId> = self.panes.keys().copied().collect();
        pane_ids.sort();
        for pane_id in pane_ids {
            let p = self.panes[&pane_id];
            let parser = take_parser(pane_id);
            let per_paint = |n: u64| {
                if p.paints == 0 {
                    0.
                } else {
                    n as f64 / p.paints as f64
                }
            };
            let misses = p.miss_moved_or_context + p.miss_content + p.miss_expired_or_hover;
            let _ = writeln!(
                out,
                "  pane {pane_id}: paints={} rows/paint={:.1} clean_rows/paint={:.1} | \
                 lock_held ms/paint={:.3} ({:.1}% of wall)",
                p.paints,
                per_paint(p.rows),
                per_paint(p.clean_rows),
                per_paint(p.lock_held_ns) / 1e6,
                p.lock_held_ns as f64 / 1e7 / secs,
            );
            let _ = writeln!(
                out,
                "    quad cache hits={} misses={} (moved_or_context={} content={} \
                 expired_or_hover={}) | hit ms/paint={:.3} quads_copied={} | \
                 miss ms/paint={:.3} quads_built={}",
                p.quad_hits,
                misses,
                p.miss_moved_or_context,
                p.miss_content,
                p.miss_expired_or_hover,
                per_paint(p.hit_ns) / 1e6,
                p.quads_copied_on_hit,
                per_paint(p.miss_ns) / 1e6,
                p.quads_built,
            );
            let _ = writeln!(
                out,
                "    harfbuzz calls={} ms/paint={:.3} | parser flushes/s={:.1} actions/s={:.0} \
                 perform_actions ms={:.2} ({:.1}% of wall) max={:.2}ms",
                p.harfbuzz_calls,
                per_paint(p.harfbuzz_ns) / 1e6,
                rate(parser.flushes),
                rate(parser.actions),
                ms(parser.perform_ns),
                parser.perform_ns as f64 / 1e7 / secs,
                ms(parser.perform_max_ns),
            );
        }

        log::info!("{}", out.trim_end());

        let prev_max_seqno = std::mem::take(&mut self.prev_max_seqno);
        *self = Self::default();
        self.prev_max_seqno = prev_max_seqno;
    }
}
