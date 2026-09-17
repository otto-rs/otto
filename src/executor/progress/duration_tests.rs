#![cfg(test)]

use super::format_task_duration;
use std::time::Duration;

#[test]
fn sub_minute_durations_read_as_bare_seconds() {
    assert_eq!(format_task_duration(Duration::from_millis(0)), "0s");
    assert_eq!(format_task_duration(Duration::from_millis(999)), "0s");
    assert_eq!(format_task_duration(Duration::from_secs(8)), "8s");
    assert_eq!(format_task_duration(Duration::from_secs(59)), "59s");
}

#[test]
fn minutes_pad_their_seconds_so_the_column_does_not_jump() {
    assert_eq!(format_task_duration(Duration::from_secs(60)), "1m00s");
    assert_eq!(format_task_duration(Duration::from_secs(114)), "1m54s");
    assert_eq!(format_task_duration(Duration::from_secs(245)), "4m05s");
    assert_eq!(format_task_duration(Duration::from_secs(2482)), "41m22s");
}

#[test]
fn hours_keep_both_lower_fields() {
    assert_eq!(format_task_duration(Duration::from_secs(3600)), "1h00m00s");
    assert_eq!(format_task_duration(Duration::from_secs(7511)), "2h05m11s");
}

/// Truncation, not rounding: a row that says `2s` when 2.9s have passed is
/// behind the truth, which is harmless; one that says `3s` at 2.1s has claimed
/// time that has not happened.
#[test]
fn a_partial_second_is_never_rounded_up() {
    assert_eq!(format_task_duration(Duration::from_millis(2_900)), "2s");
    assert_eq!(format_task_duration(Duration::from_millis(59_999)), "59s");
    assert_eq!(format_task_duration(Duration::from_millis(3_599_999)), "59m59s");
}
