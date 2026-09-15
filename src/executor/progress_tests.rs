//! Tests for the idle-triggered spinner.
//!
//! Everything here is a pure function on purpose. The gate takes its
//! environment as a closure and the frame takes its width as an argument, so
//! the rules that matter -- when a spinner is allowed to exist, and what it is
//! allowed to write -- are testable without a pty, without `set_var`, and
//! without racing a ticker thread.

use super::*;

/// The enabled-when-a-terminal rule, and every reason to override it.
///
/// The first row is the promise this feature makes: piped stderr never spins,
/// which is what keeps frames out of a log file.
#[test]
fn gate_rules() {
    let none = |_: &str| None;
    let term = |k: &str| (k == "TERM").then(|| "xterm-256color".to_string());

    assert!(!should_enable(false, false, false, term), "piped stderr never spins");
    assert!(should_enable(true, false, false, term), "a terminal with TERM set does");

    assert!(!should_enable(true, true, false, term), "--tui owns the screen");
    assert!(!should_enable(true, false, true, term), "--no-progress is the opt-out");
    assert!(
        !should_enable(true, false, false, none),
        "TERM unset is not a usable terminal"
    );

    for (var, value) in [
        ("CI", "true"),
        ("NO_COLOR", "1"),
        ("OTTO_NO_PROGRESS", "1"),
        ("TERM", "dumb"),
    ] {
        let env = move |k: &str| {
            if k == var {
                Some(value.to_string())
            } else if k == "TERM" {
                Some("xterm-256color".to_string())
            } else {
                None
            }
        };
        assert!(
            !should_enable(true, false, false, env),
            "{var}={value} must disable the spinner"
        );
    }

    // Set-but-empty is not set: `CI=` in an env block should not disable it.
    let empty_ci = |k: &str| match k {
        "CI" => Some(String::new()),
        "TERM" => Some("xterm-256color".to_string()),
        _ => None,
    };
    assert!(should_enable(true, false, false, empty_ci), "an empty CI is not CI");
}

/// A frame must fit on one row. A wrapped frame occupies two rows while the
/// erase clears one, and the leftover half-line is precisely the defaced
/// output this feature is not allowed to produce.
#[test]
fn frame_never_exceeds_its_width() {
    let many: Vec<String> = (0..20).map(|i| format!("service-with-a-long-name-{i}")).collect();
    for width in [10usize, 20, 40, 80, 200] {
        let line = frame_line("⠙", &many, Duration::from_secs(90), width);
        assert!(
            line.chars().count() <= width.saturating_sub(1),
            "width {width}: {:?} is {} chars, budget {}",
            line,
            line.chars().count(),
            width - 1
        );
    }
}

/// Degenerate widths must produce nothing rather than panic or emit a stray
/// character: a terminal that reports zero columns still must not be scribbled on.
#[test]
fn frame_copes_with_no_room() {
    for width in [0usize, 1] {
        assert_eq!(frame_line("⠙", &["philo".to_string()], Duration::ZERO, width), "");
    }
}

/// The frame says what is taking the time, and for how long -- both halves of
/// what was asked for ("a little ascii spinner", "or even just a timer counting").
#[test]
fn frame_names_the_work_and_the_clock() {
    let one = frame_line("⠙", &["philo".to_string()], Duration::from_secs(62), 80);
    assert!(one.contains("philo"), "{one}");
    assert!(one.contains("1:02"), "{one}");

    let two = frame_line(
        "⠙",
        &["philo".to_string(), "catalog-api".to_string()],
        Duration::from_secs(5),
        80,
    );
    assert!(two.contains('2'), "several tasks are counted: {two}");
    assert!(two.contains("philo") && two.contains("catalog-api"), "{two}");
    assert!(two.contains("0:05"), "{two}");
}

/// A frame carries no newline, ever. That is the property that keeps it out of
/// scrollback: a line that is never terminated cannot scroll away.
#[test]
fn frame_has_no_newline() {
    let line = frame_line("⠙", &["philo".to_string()], Duration::from_secs(1), 80);
    assert!(!line.contains('\n'), "{line:?}");
    assert!(!line.contains('\r'), "{line:?}");
}
