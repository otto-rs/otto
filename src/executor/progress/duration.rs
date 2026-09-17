//! The one duration format a live row and a completion line both use.
//!
//! One function, in one place, because the two surfaces have to agree: Phase 5
//! of `docs/design/2026-09-16-live-progress-renderer.md` exists partly because
//! a row reading `running 1s` beside a completion line reading `10m01s` is the
//! defect, and two formatters are how a fixed measurement drifts apart again in
//! presentation instead.
//!
//! Deliberately not `cli/commands/format.rs`'s `format_duration`, which the
//! deleted heartbeat used: that one renders sub-minute values as `41.0s` and
//! milliseconds as `200ms`, which is right for a history table of finished runs
//! and wrong for a row that is redrawn five times a second, where a tenth of a
//! second of churn is noise.

use std::time::Duration;

/// A task duration, in the shape the design doc's mockups use: `8s`, `1m54s`,
/// `2h05m11s`. Truncated toward zero, never rounded up, so a row can never
/// claim more elapsed time than has passed.
pub fn format_task_duration(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    let (hours, minutes, seconds) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if hours > 0 {
        format!("{hours}h{minutes:02}m{seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m{seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

#[path = "duration_tests.rs"]
mod tests;
