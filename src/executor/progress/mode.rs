//! The `--progress` mode: what the invocation asked for
//! ([`ProgressSetting`]), and what that resolves to once stderr is inspected
//! ([`ProgressMode`]). Resolved exactly once, at startup, and never
//! re-derived: npm shipped the bug where the same predicate, computed twice,
//! diverged (`npm/cli` `5b858c6`). See
//! `docs/design/2026-09-16-live-progress-renderer.md`, API Design.
//!
//! No renderer consumes [`ProgressMode`] yet - Phase 3 of that design only
//! computes and plumbs it. Phase 5 adds the region behind it.

use std::env;
use std::ffi::OsString;
use std::io::IsTerminal;

/// What `--progress` / `OTTO_PROGRESS` / `otto.progress` asked for, before
/// stderr is consulted.
///
/// Never `always`. Cargo's `term.progress.when` has a third value, and this
/// design started out offering it too, but Phase 0 measured it and cut it:
/// indicatif 0.18.3's `TargetKind::TermLike` draw path has no predicate for
/// "is this a terminal" to fake, but otto still cannot supply a correct
/// width/height (`TIOCGWINSZ` on a non-terminal fd is `-1`) or a cursor erase
/// (a cursor op into a pipe is only more bytes) for a stream that is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressSetting {
    Auto,
    Never,
}

impl ProgressSetting {
    /// The only spellings `--progress`, `OTTO_PROGRESS`, and `otto.progress`
    /// accept. One list, so all three name the same set in their error text
    /// instead of three lists that can drift.
    pub const VALID: &'static [&'static str] = &["auto", "never"];

    /// Parses one of [`VALID`](Self::VALID). The CLI's own value is validated
    /// by clap's `PossibleValuesParser` before this is ever called (see
    /// `cli/parser/help.rs`'s `--progress` arg), so in practice this is
    /// reached only from `otto.progress` in an ottofile. Kept as a plain
    /// `Result<_, String>`, not a `clap`/`serde` error type, so it has no
    /// caller-specific framing baked in; each caller wraps it in its own
    /// context.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "never" => Ok(Self::Never),
            other => Err(format!(
                "invalid progress value '{other}': expected one of {} \
                 ('always' was measured and cut, see docs/design/2026-09-16-live-progress-renderer.md)",
                Self::VALID.join(", ")
            )),
        }
    }
}

/// Whether this run draws a live region on stderr. Computed ONCE at startup
/// from a [`ProgressSetting`] and stderr's own state, then stored - never
/// re-derived mid-run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    Live,
    Quiet,
}

impl ProgressMode {
    /// `auto` is Live iff stderr is a terminal, AND `TERM` is not `dumb`, AND
    /// no `CI` env var is set. `never` is always Quiet.
    ///
    /// otto owns the `TERM=dumb` check itself: Phase 0 decided to stay on
    /// indicatif 0.18.3, whose own `is_term()` check has no dumb-terminal
    /// predicate (0.18.6 adds `is_dumb()`, but bumping is a two-crate move
    /// this design does not take - see Technical Considerations).
    pub fn resolve(setting: ProgressSetting) -> Self {
        Self::resolve_with(
            setting,
            std::io::stderr().is_terminal(),
            env::var_os("TERM"),
            env::var_os("CI").is_some(),
        )
    }

    /// The pure predicate behind [`resolve`](Self::resolve), taking the three
    /// environment facts as arguments instead of reading them itself, so a
    /// test can assert every combination without touching the real terminal
    /// or process environment.
    fn resolve_with(setting: ProgressSetting, stderr_is_terminal: bool, term: Option<OsString>, ci: bool) -> Self {
        match setting {
            ProgressSetting::Never => Self::Quiet,
            ProgressSetting::Auto => {
                let dumb = term.as_deref() == Some(std::ffi::OsStr::new("dumb"));
                if stderr_is_terminal && !dumb && !ci { Self::Live } else { Self::Quiet }
            }
        }
    }
}

#[path = "mode_tests.rs"]
mod tests;
