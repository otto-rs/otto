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
///
/// Inverted from `a_short_terminal_is_clamped_below_the_floor`, which pinned
/// `row_cap(3) == 2`. Two task rows plus the separator plus the overflow row is
/// four lines in a three-line terminal, so indicatif dropped the overflow row -
/// the exact row the cap was written to protect. The clamp reserves both chrome
/// rows now, so every height below leaves them a line each.
#[test]
fn a_short_terminal_still_leaves_the_separator_and_the_overflow_row_a_line_each() {
    assert_eq!(row_cap(9), MIN_ROWS);
    for height in 1..=12u16 {
        let cap = row_cap(height);
        assert!(
            cap >= 1,
            "a region with no task row in it says nothing: height {height}"
        );
        assert!(
            cap + CHROME_ROWS as usize <= (height as usize).max(3),
            "row_cap({height}) == {cap} asks for {} lines in a {height}-line terminal",
            cap + CHROME_ROWS as usize
        );
    }
    assert_eq!(row_cap(3), 1);
    assert_eq!(row_cap(4), 2);
    assert_eq!(row_cap(5), 3);
    assert_eq!(row_cap(1), 1);
    assert_eq!(row_cap(0), 1);
}

// ---------------------------------------------------------------------------
// Resize: the cap is re-read while a run is in flight, not only when a task
// starts.
// ---------------------------------------------------------------------------

/// A terminal that shrinks must give rows back, and one that grows must take
/// them again. Before this, `self.cap()` was read in exactly one place -
/// `started` - so a populated region kept every row it had through a resize:
/// shrinking left six rows attached in a three-row window with no overflow row,
/// and growing promoted nobody until a task happened to finish.
#[test]
fn a_resize_moves_rows_between_the_screen_and_the_overflow_row() {
    let mut region = Region::with_cap(&names(&["a", "b", "c", "d"]), 4);
    for task in ["a", "b", "c", "d"] {
        region.started(task, clock(task));
    }
    assert_eq!(region.row_labels(), vec!["a", "b", "c", "d"]);
    assert!(!region.has_overflow_row(), "four rows under a cap of four hide nothing");

    region.set_cap(2);
    region.refresh();
    assert_eq!(region.row_labels(), vec!["a", "b"], "a shrink must demote from the end");
    assert_eq!(region.waiting_len(), 2);
    assert!(
        region.has_overflow_row(),
        "the rows a shrink took off the screen still have to be counted"
    );

    region.set_cap(4);
    region.refresh();
    assert_eq!(
        region.row_labels(),
        vec!["a", "b", "c", "d"],
        "a regrow must promote the same rows back in the same order"
    );
    assert_eq!(region.waiting_len(), 0);
    assert!(!region.has_overflow_row(), "nothing is hidden any more");
}

// ---------------------------------------------------------------------------
// The panic boundary.
// ---------------------------------------------------------------------------

/// A terminal whose every operation fails, which is the only way to reach
/// indicatif's own unwraps: `MultiState::suspend` is `clear().unwrap(); f();
/// draw().unwrap()`, and both halves return `io::Result`.
#[derive(Debug)]
struct BrokenTerm;

impl BrokenTerm {
    fn hung_up() -> std::io::Error {
        std::io::Error::other("deliberate: the terminal hung up mid-draw")
    }
}

impl indicatif::TermLike for BrokenTerm {
    fn width(&self) -> u16 {
        80
    }

    fn height(&self) -> u16 {
        24
    }

    fn move_cursor_up(&self, _: usize) -> std::io::Result<()> {
        Err(Self::hung_up())
    }

    fn move_cursor_down(&self, _: usize) -> std::io::Result<()> {
        Err(Self::hung_up())
    }

    fn move_cursor_right(&self, _: usize) -> std::io::Result<()> {
        Err(Self::hung_up())
    }

    fn move_cursor_left(&self, _: usize) -> std::io::Result<()> {
        Err(Self::hung_up())
    }

    fn write_line(&self, _: &str) -> std::io::Result<()> {
        Err(Self::hung_up())
    }

    fn write_str(&self, _: &str) -> std::io::Result<()> {
        Err(Self::hung_up())
    }

    fn clear_line(&self) -> std::io::Result<()> {
        Err(Self::hung_up())
    }

    fn flush(&self) -> std::io::Result<()> {
        Err(Self::hung_up())
    }
}

/// The design's promise: "indicatif unwraps; the facade catches, so a render
/// failure degrades to plain writes instead of poisoning every later writer."
///
/// Three claims in one, because they are one behaviour: the panic does not
/// escape, the suspended write still happens EXACTLY once, and the renderer
/// stays retired afterwards rather than panicking again on every later call
/// against indicatif's own poisoned lock.
#[test]
fn a_renderer_panic_is_caught_and_the_write_still_happens_exactly_once() {
    let target = indicatif::ProgressDrawTarget::term_like(Box::new(BrokenTerm));
    let mut region = Region::with_draw_target(&names(&["a"]), target);
    region.started("a", clock("a"));
    assert!(
        region.is_drawn(),
        "the fixture needs a drawn region, or `suspend` short-circuits and proves nothing"
    );

    let mut writes = 0;
    let value = region.suspend(|| {
        writes += 1;
        "the plain write"
    });
    assert_eq!(value, "the plain write", "the closure's value must still come back");
    assert_eq!(
        writes, 1,
        "the suspended write must run once: not zero (output lost to a render failure) \
         and not twice (a panic after the write re-running it)"
    );
    assert!(
        region.is_retired(),
        "a renderer that panicked must be retired; indicatif does not recover its own poisoned lock"
    );

    let mut later = 0;
    region.suspend(|| later += 1);
    assert_eq!(later, 1, "a retired region still writes, plainly");
    assert!(
        !region.is_drawn(),
        "a retired region has nothing on the screen to suspend"
    );
    // None of these may panic either: every one of them reaches indicatif.
    region.refresh();
    region.erase();
    region.rearm();
    region.finished("a");
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
