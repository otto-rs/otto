#![cfg(test)]

use super::*;

#[test]
fn test_consistent_color_assignment() {
    // Same task name should always get same color
    let color1 = get_task_color("build");
    let color2 = get_task_color("build");
    assert_eq!(color1, color2);

    // Test that we're cycling through our expected range
    for i in 0..16 {
        let task_name = format!("task_{i}");
        let color = get_task_color(&task_name);
        // Just ensure we can get a color without panicking
        let _ = format!("{color:?}");
    }
}

#[test]
fn test_colorize_functions() {
    let task_name = "test_task";

    // These should not panic and should return strings
    let colored_name = colorize_task_name(task_name);
    let colored_prefix = colorize_task_prefix(task_name);

    assert!(colored_name.contains("test_task"));
    // The colored prefix contains ANSI escape codes, so we need to check for the task name
    // and the brackets separately, or check that it contains the task name
    assert!(colored_prefix.contains("test_task"));
    assert!(colored_prefix.contains("["));
    assert!(colored_prefix.contains("]"));
}

#[test]
fn test_get_task_color_combination() {
    // Test direct color combination retrieval
    let (bracket, text) = get_task_color_combination("some_task");
    // Bracket and text colors should be different
    assert_ne!(format!("{bracket:?}"), format!("{text:?}"));
}

#[test]
fn test_set_global_task_order() {
    // Test that setting global task order doesn't panic
    set_global_task_order(vec!["z".to_string(), "a".to_string(), "m".to_string()]);

    // Get color for a task after setting order
    let (bracket, text) = get_task_color_combination("a");
    let _ = format!("{bracket:?} {text:?}");
}

#[test]
fn test_color_combinations_count() {
    // Verify we have 15 unique color combinations
    assert_eq!(COLOR_COMBINATIONS.len(), 15);

    // Verify all bracket and text colors are different in each combination
    for (bracket, text) in COLOR_COMBINATIONS.iter() {
        assert_ne!(format!("{bracket:?}"), format!("{text:?}"));
    }
}

/// `stream_task_label` is the selector every stderr-writing site goes through,
/// and it must pick by the stream, not by `colored`'s stdout-derived answer.
/// Asserted against both label functions rather than against literal text, so
/// it holds whether or not the test process has a terminal.
#[test]
fn stream_task_label_picks_the_plain_form_for_a_stream_that_takes_no_colour() {
    assert_eq!(
        stream_task_label("build", false, false),
        plain_task_label("build", false)
    );
    assert_eq!(stream_task_label("build", false, true), task_label("build", false));
}

/// The plain form still honours `--no-prefix`: the flag and the colour decision
/// are independent, and a no-prefix run writing to a redirected stderr gets a
/// bare task name with no escapes.
#[test]
fn stream_task_label_keeps_no_prefix_independent_of_colour() {
    assert_eq!(stream_task_label("build", true, false), "build");
    assert!(!stream_task_label("build", true, false).contains('\u{1b}'));
    assert_eq!(stream_task_label("build", false, false), "[build]");
}

/// Read once for the process: the heartbeat reads this at spawn and every
/// status line reads it per line, and the two must never disagree (npm's
/// commit `5b858c6` shipped a progress predicate whose two readings diverged).
#[test]
fn stderr_takes_color_answers_the_same_every_time() {
    assert_eq!(stderr_takes_color(), stderr_takes_color());
}

/// `CLICOLOR_FORCE` forces colour onto a stderr that is not a terminal, which
/// is the whole point of the variable and what the terminal-only predicate
/// vetoed. Tested through the pure form: the suite is process-shared and cannot
/// set an environment variable for one test.
#[test]
fn clicolor_force_makes_a_non_terminal_stderr_take_colour() {
    assert!(stderr_takes_color_from(false, Some("1")));
    assert!(!stderr_takes_color_from(false, None));
}

/// `colored`'s `normalize_env` rule, not a truthiness rule of otto's own: only
/// the exact value `0` declines, so `CLICOLOR_FORCE=yes` cannot colour otto's
/// stdout and not its stderr.
#[test]
fn only_the_value_zero_declines_the_force() {
    assert!(!stderr_takes_color_from(false, Some("0")));
    assert!(stderr_takes_color_from(false, Some("yes")));
    assert!(stderr_takes_color_from(false, Some("")));
}

/// A terminal stderr still takes colour with the force set to `0`: in `colored`
/// that value resolves to no override and falls through to the tty check, so
/// treating it as a suppressor would diverge.
#[test]
fn a_terminal_stderr_is_not_suppressed_by_a_zero_force() {
    assert!(stderr_takes_color_from(true, Some("0")));
    assert!(stderr_takes_color_from(true, None));
}
