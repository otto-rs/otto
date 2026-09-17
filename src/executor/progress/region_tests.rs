#![cfg(test)]

use super::*;
use crate::executor::clocks::TaskClocks;

/// A clock for `task`, started now. The region holds these; it never stamps a
/// start instant of its own.
fn clock(task: &str) -> Arc<TaskClock> {
    Box::leak(Box::new(TaskClocks::default())).start(task)
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

// ---------------------------------------------------------------------------
// The row cap.
// ---------------------------------------------------------------------------

#[test]
fn the_row_cap_leaves_room_for_the_overflow_row_and_the_prompt() {
    assert_eq!(row_cap(24), 18);
    assert_eq!(row_cap(50), 44);
}

/// The floor and the clamp are two different guards and a short terminal needs
/// both: `max(3, ...)` would ask a 3-row terminal for 3 rows plus a separator
/// plus an overflow row.
#[test]
fn a_short_terminal_is_clamped_below_the_floor() {
    assert_eq!(row_cap(9), MIN_ROWS);
    assert_eq!(row_cap(3), 2);
    assert_eq!(row_cap(1), 1);
    assert_eq!(row_cap(0), 1);
}

// ---------------------------------------------------------------------------
// Bounded rows.
// ---------------------------------------------------------------------------

#[test]
fn rows_past_the_cap_are_counted_by_one_overflow_row() {
    let mut region = Region::with_cap(&names(&["a", "b", "c", "d", "e"]), 2);
    for task in ["a", "b"] {
        region.started(task, clock(task));
    }
    assert_eq!(region.row_labels(), vec!["a", "b"]);
    assert!(
        !region.has_overflow_row(),
        "two rows under a cap of two need no overflow"
    );

    for task in ["c", "d", "e"] {
        region.started(task, clock(task));
    }
    assert_eq!(region.row_labels(), vec!["a", "b"], "the cap must hold");
    assert_eq!(region.waiting_len(), 3);
    assert!(region.has_overflow_row(), "three hidden tasks need the overflow row");
}

/// The design doc's reason for `mp.remove` being explicit: indicatif's own
/// `mark_zombie` reaps only the LEADING run of finished bars, so a task that
/// finishes in the middle would leave a dead row. Asserted through the row
/// list, which is what the screen is drawn from.
#[test]
fn a_task_finishing_in_the_middle_gives_its_row_up_to_a_waiting_task() {
    let mut region = Region::with_cap(&names(&["a", "b", "c", "d"]), 3);
    for task in ["a", "b", "c", "d"] {
        region.started(task, clock(task));
    }
    assert_eq!(region.row_labels(), vec!["a", "b", "c"]);
    assert_eq!(region.waiting_len(), 1);

    region.finished("b");
    assert_eq!(region.row_labels(), vec!["a", "c", "d"], "d must take the freed row");
    assert_eq!(region.waiting_len(), 0);
    assert!(!region.has_overflow_row(), "nothing is hidden any more");
}

#[test]
fn a_task_that_finishes_while_waiting_is_dropped_from_the_count() {
    let mut region = Region::with_cap(&names(&["a", "b", "c"]), 1);
    for task in ["a", "b", "c"] {
        region.started(task, clock(task));
    }
    assert_eq!(region.waiting_len(), 2);
    region.finished("c");
    assert_eq!(region.waiting_len(), 1);
    assert_eq!(
        region.row_labels(),
        vec!["a"],
        "a waiting task finishing promotes nobody"
    );
}

#[test]
fn the_last_row_finishing_leaves_nothing_drawn() {
    let mut region = Region::new(&names(&["only"]));
    assert!(!region.is_drawn(), "a region with no rows has nothing on the screen");
    region.started("only", clock("only"));
    assert!(region.is_drawn());
    region.finished("only");
    assert!(!region.is_drawn(), "the separator goes with the last row");
}

#[test]
fn a_task_started_twice_gets_one_row() {
    let mut region = Region::new(&names(&["a"]));
    region.started("a", clock("a"));
    region.started("a", clock("a"));
    assert_eq!(region.row_labels(), vec!["a"]);
}

// ---------------------------------------------------------------------------
// Row text.
// ---------------------------------------------------------------------------

#[test]
fn a_row_names_its_task_and_how_long_it_has_run() {
    let row = Row {
        name: "philo".to_string(),
        label: "philo".to_string(),
        clock: clock("philo"),
    };
    let text = row_text(&row, 8, 80, false);
    assert!(
        text.starts_with("philo     running "),
        "expected a padded name and the elapsed figure, got {text:?}"
    );
}

/// The idle clause appears only past the threshold. Below it the number churns
/// faster than it can be read and every row would carry one.
#[test]
fn a_row_says_nothing_about_silence_until_the_silence_is_worth_saying() {
    let row = Row {
        name: "quiet".to_string(),
        label: "quiet".to_string(),
        clock: clock("quiet"),
    };
    assert!(
        !row_text(&row, 5, 80, false).contains("no output for"),
        "a task that has just started is not yet interestingly silent"
    );
}

#[test]
fn a_row_is_cut_to_one_column_short_of_the_terminal() {
    let row = Row {
        name: "x".repeat(200),
        label: "x".repeat(200),
        clock: clock("wide"),
    };
    let text = row_text(&row, 0, 40, false);
    assert_eq!(console::measure_text_width(&text), 40);
}

// ---------------------------------------------------------------------------
// Truncation and sanitization.
// ---------------------------------------------------------------------------

#[test]
fn truncation_measures_columns_not_bytes() {
    assert_eq!(truncate("abcdef", 3), "abc");
    assert_eq!(truncate("abc", 10), "abc");
    // A wide glyph costs two columns, so three of them do not fit in five.
    assert_eq!(truncate("\u{5b9d}\u{5b9d}\u{5b9d}", 5), "\u{5b9d}\u{5b9d}");
}

/// A grapheme boundary, not a `char` boundary: `e` plus a combining acute is
/// two `char`s and one cluster, and cutting between them leaves a bare
/// combining mark that the terminal hangs on whatever precedes it.
#[test]
fn truncation_never_splits_a_grapheme_cluster() {
    let text = "e\u{301}e\u{301}e\u{301}";
    assert_eq!(truncate(text, 2), "e\u{301}e\u{301}");
    assert_eq!(truncate(text, 1), "e\u{301}");
}

#[test]
fn sanitizing_drops_control_bytes_and_expands_tabs() {
    assert_eq!(sanitize("a\tb"), "a    b");
    assert_eq!(sanitize("a\rb\nc\x07d"), "abcd");
    assert_eq!(sanitize("plain"), "plain");
}

/// The only SGR on a row is the SGR otto put there. A task name carrying an
/// escape sequence would otherwise recolour, move, or erase otto's own region.
#[test]
fn sanitizing_strips_escape_sequences_whole() {
    assert_eq!(sanitize("\x1b[31mred\x1b[0m"), "red");
    assert_eq!(sanitize("\x1b[2Jwipe"), "wipe");
    assert_eq!(sanitize("\x1bMreverse-index"), "reverse-index");
    assert_eq!(sanitize("keep\x1b["), "keep");
}

#[test]
fn a_task_name_carrying_an_escape_cannot_reach_a_row() {
    let mut region = Region::new(&names(&["\x1b[2Jbad"]));
    region.started("\x1b[2Jbad", clock("bad"));
    assert_eq!(region.row_labels(), vec!["bad"]);
}

// ---------------------------------------------------------------------------
// Colour.
// ---------------------------------------------------------------------------

#[test]
fn a_row_takes_no_colour_when_stderr_takes_none() {
    assert_eq!(colorize_name("alpha  running 3s", "alpha", false), "alpha  running 3s");
}

/// Colour goes on the name and nowhere else, and only when the name survived
/// truncation whole: an SGR opened inside a cut name is an SGR with no reset.
#[test]
fn colour_is_skipped_when_truncation_ate_the_name() {
    let cut = "alp";
    assert_eq!(colorize_name(cut, "alpha", true), cut);
}
