//! Application-wide motion cadence for small recurring UI animations.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{AsyncApp, Context, EntityId, Global};

const MOTION_TICK: Duration = Duration::from_millis(33);
const MOTION_LEASE: Duration = Duration::from_millis(250);

struct MotionClock {
    epoch: Instant,
    leases: HashMap<EntityId, Instant>,
    running: bool,
}

impl Default for MotionClock {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            leases: HashMap::new(),
            running: false,
        }
    }
}

impl Global for MotionClock {}

fn phase_at(elapsed: Duration, period: Duration) -> f32 {
    if period.is_zero() {
        return 0.0;
    }
    (elapsed.as_secs_f64() / period.as_secs_f64()).fract() as f32
}

/// Return a stable phase while one shared 30 Hz clock invalidates every view
/// currently rendering recurring motion. Reduced-motion mode never leases the
/// clock, so no background repaint loop remains alive.
pub(crate) fn repeating_phase<V: 'static>(period: Duration, cx: &mut Context<V>) -> f32 {
    if cx.prefers_reduced_motion() {
        return 0.0;
    }

    let now = Instant::now();
    let entity_id = cx.entity_id();
    let (phase, start_clock) = {
        let clock = cx.default_global::<MotionClock>();
        clock.leases.insert(entity_id, now);
        let phase = phase_at(now.saturating_duration_since(clock.epoch), period);
        let start_clock = !clock.running;
        clock.running = true;
        (phase, start_clock)
    };

    if start_clock {
        cx.spawn(async move |_view, cx: &mut AsyncApp| loop {
            cx.background_executor().timer(MOTION_TICK).await;
            let Ok(active) = cx.update(|cx| {
                let now = Instant::now();
                let entities = {
                    let clock = cx.default_global::<MotionClock>();
                    clock
                        .leases
                        .retain(|_, renewed| now.duration_since(*renewed) <= MOTION_LEASE);
                    if clock.leases.is_empty() {
                        clock.running = false;
                    }
                    clock.leases.keys().copied().collect::<Vec<_>>()
                };
                for entity in &entities {
                    cx.notify(*entity);
                }
                !entities.is_empty()
            }) else {
                break;
            };
            if !active {
                break;
            }
        })
        .detach();
    }

    phase
}

#[cfg(test)]
mod tests {
    use super::phase_at;
    use std::time::Duration;

    // SDTEST-1690
    #[test]
    fn repeating_phase_wraps_at_the_shared_period() {
        let period = Duration::from_millis(1_200);
        assert_eq!(phase_at(Duration::ZERO, period), 0.0);
        assert!((phase_at(Duration::from_millis(300), period) - 0.25).abs() < f32::EPSILON);
        assert_eq!(phase_at(period, period), 0.0);
        assert!((phase_at(Duration::from_millis(1_500), period) - 0.25).abs() < f32::EPSILON);
    }
}
