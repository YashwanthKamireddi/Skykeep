//! How long pictures and sound take from the phone to this computer's programs.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

/// Microseconds since 1970 on this computer's clock: what the phone's capture times are set
/// against once the two clocks are matched, and what the test phone stamps directly.
pub fn now_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

/// The delays of the last few hundred pictures or sound blocks.
#[derive(Default)]
pub struct Delays {
    recent: VecDeque<i64>,
    pub count: u64,
}

impl Delays {
    const KEPT: usize = 600;

    pub fn record(&mut self, captured_us: i64) {
        self.recent.push_back(now_us() - captured_us);
        if self.recent.len() > Self::KEPT {
            self.recent.pop_front();
        }
        self.count += 1;
    }

    /// The given share of recent delays are at or under the answer, in microseconds.
    pub fn percentile(&self, share: f64) -> Option<i64> {
        if self.recent.is_empty() {
            return None;
        }
        let mut sorted: Vec<i64> = self.recent.iter().copied().collect();
        sorted.sort_unstable();
        Some(sorted[((sorted.len() - 1) as f64 * share).round() as usize])
    }
}
