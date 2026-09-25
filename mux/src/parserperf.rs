//! Per-pane parser counters for render-latency profiling.
//! Only collected when `WEZTERM_RENDER_STATS=1` is set at startup.
use crate::pane::PaneId;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::OnceLock;

pub fn render_stats_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("WEZTERM_RENDER_STATS").as_deref() == Ok("1"))
}

#[derive(Default, Debug, Clone, Copy)]
pub struct ParserPerf {
    /// Number of send_actions_to_mux flushes
    pub flushes: u64,
    /// Number of parsed actions applied
    pub actions: u64,
    /// Time spent in Pane::perform_actions, including waiting
    /// for the terminal lock
    pub perform_ns: u64,
    pub perform_max_ns: u64,
}

fn stats() -> &'static Mutex<HashMap<PaneId, ParserPerf>> {
    static STATS: OnceLock<Mutex<HashMap<PaneId, ParserPerf>>> = OnceLock::new();
    STATS.get_or_init(Default::default)
}

/// Called once per flush from the parser thread
pub(crate) fn record(pane_id: PaneId, actions: usize, perform_ns: u64) {
    let mut stats = stats().lock();
    let entry = stats.entry(pane_id).or_default();
    entry.flushes += 1;
    entry.actions += actions as u64;
    entry.perform_ns += perform_ns;
    entry.perform_max_ns = entry.perform_max_ns.max(perform_ns);
}

/// Returns and resets the accumulated counters for a pane
pub fn take(pane_id: PaneId) -> ParserPerf {
    stats().lock().remove(&pane_id).unwrap_or_default()
}
