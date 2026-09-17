#![cfg(test)]

use std::ffi::OsString;

use super::{ProgressMode, ProgressSetting};

#[test]
fn parse_accepts_auto_and_never() {
    assert_eq!(ProgressSetting::parse("auto"), Ok(ProgressSetting::Auto));
    assert_eq!(ProgressSetting::parse("never"), Ok(ProgressSetting::Never));
}

/// `always` is the one spelling a user may have read about (cargo's
/// `term.progress.when` has it) and it must fail with a message naming what
/// IS valid, not a panic and not a silent fallback to `auto`.
#[test]
fn parse_rejects_always_by_name() {
    let err = ProgressSetting::parse("always").unwrap_err();
    assert!(err.contains("always"), "error should name the rejected value: {err}");
    assert!(err.contains("auto"), "error should name a valid value: {err}");
    assert!(err.contains("never"), "error should name a valid value: {err}");
}

#[test]
fn parse_rejects_garbage() {
    assert!(ProgressSetting::parse("").is_err());
    assert!(
        ProgressSetting::parse("Auto").is_err(),
        "case must match exactly, like clap's own values"
    );
}

#[test]
fn never_is_always_quiet() {
    assert_eq!(
        ProgressMode::resolve_with(ProgressSetting::Never, true, None, false),
        ProgressMode::Quiet
    );
    assert_eq!(
        ProgressMode::resolve_with(
            ProgressSetting::Never,
            true,
            Some(OsString::from("xterm-256color")),
            false
        ),
        ProgressMode::Quiet
    );
}

#[test]
fn auto_is_live_on_a_plain_terminal() {
    assert_eq!(
        ProgressMode::resolve_with(
            ProgressSetting::Auto,
            true,
            Some(OsString::from("xterm-256color")),
            false
        ),
        ProgressMode::Live
    );
}

#[test]
fn auto_is_quiet_off_a_terminal() {
    assert_eq!(
        ProgressMode::resolve_with(
            ProgressSetting::Auto,
            false,
            Some(OsString::from("xterm-256color")),
            false
        ),
        ProgressMode::Quiet
    );
}

#[test]
fn auto_is_quiet_on_term_dumb() {
    assert_eq!(
        ProgressMode::resolve_with(ProgressSetting::Auto, true, Some(OsString::from("dumb")), false),
        ProgressMode::Quiet
    );
}

#[test]
fn auto_is_live_with_no_term_set() {
    // `TERM != dumb` is the spec; an unset `TERM` is not the string `dumb`,
    // so it does not gate. Only the literal value `dumb` does.
    assert_eq!(
        ProgressMode::resolve_with(ProgressSetting::Auto, true, None, false),
        ProgressMode::Live
    );
}

#[test]
fn auto_is_quiet_under_ci_even_on_a_tty() {
    assert_eq!(
        ProgressMode::resolve_with(
            ProgressSetting::Auto,
            true,
            Some(OsString::from("xterm-256color")),
            true
        ),
        ProgressMode::Quiet
    );
}
