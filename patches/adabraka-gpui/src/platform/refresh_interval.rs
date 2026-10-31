use std::time::Duration;

/// Convert a display period or performance-counter ratio to a duration.
// ShellDeck patch: SDPATCH-127 — divide the full ratio instead of truncating sub-MHz frequencies to zero.
pub fn duration_from_ratio(counts: u64, frequency: u64) -> Option<Duration> {
    if counts == 0 || frequency == 0 {
        return None;
    }
    Duration::try_from_secs_f64(counts as f64 / frequency as f64).ok()
}
