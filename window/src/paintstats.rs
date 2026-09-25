//! Low-overhead WM_PAINT / throttle counters for render-latency profiling.
//! Only collected when `WEZTERM_RENDER_STATS=1` is set at startup.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// True when `WEZTERM_RENDER_STATS=1`; read once, then a single atomic load.
pub fn render_stats_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("WEZTERM_RENDER_STATS").as_deref() == Ok("1"))
}

#[derive(Default, Debug)]
pub struct PaintStats {
    pub wm_paint_received: AtomicU64,
    pub wm_paint_throttled: AtomicU64,
    pub wm_paint_dispatched: AtomicU64,
    pub throttle_count: AtomicU64,
    pub throttle_requested_ns: AtomicU64,
    pub throttle_actual_ns: AtomicU64,
    pub throttle_actual_max_ns: AtomicU64,
}

#[derive(Default, Debug, Clone, Copy)]
pub struct PaintStatsSnapshot {
    pub wm_paint_received: u64,
    pub wm_paint_throttled: u64,
    pub wm_paint_dispatched: u64,
    pub throttle_count: u64,
    pub throttle_requested_ns: u64,
    pub throttle_actual_ns: u64,
    pub throttle_actual_max_ns: u64,
}

impl PaintStats {
    pub fn add(counter: &AtomicU64, value: u64) {
        counter.fetch_add(value, Ordering::Relaxed);
    }

    /// Returns the accumulated values and resets them to zero
    pub fn take(&self) -> PaintStatsSnapshot {
        let t = |c: &AtomicU64| c.swap(0, Ordering::Relaxed);
        PaintStatsSnapshot {
            wm_paint_received: t(&self.wm_paint_received),
            wm_paint_throttled: t(&self.wm_paint_throttled),
            wm_paint_dispatched: t(&self.wm_paint_dispatched),
            throttle_count: t(&self.throttle_count),
            throttle_requested_ns: t(&self.throttle_requested_ns),
            throttle_actual_ns: t(&self.throttle_actual_ns),
            throttle_actual_max_ns: t(&self.throttle_actual_max_ns),
        }
    }
}

/// Keyed by native window handle so that the stats can be read without
/// borrowing the platform window state (which is borrowed during paint).
fn registry() -> &'static Mutex<HashMap<usize, Arc<PaintStats>>> {
    static REG: OnceLock<Mutex<HashMap<usize, Arc<PaintStats>>>> = OnceLock::new();
    REG.get_or_init(Default::default)
}

pub fn paint_stats_for(id: usize) -> Arc<PaintStats> {
    Arc::clone(registry().lock().unwrap().entry(id).or_default())
}

pub fn forget_paint_stats(id: usize) {
    registry().lock().unwrap().remove(&id);
}
