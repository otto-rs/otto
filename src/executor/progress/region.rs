//! The live region: one redrawn row per running task, on stderr, behind the
//! facade.
//!
//! Phase 5 of `docs/design/2026-09-16-live-progress-renderer.md`. Nothing here
//! is reachable except through [`super::facade::Facade`], which is what makes
//! indicatif's lock order this module's problem and nobody else's: every entry
//! point below is called with the facade's ordering lock already held, so a row
//! mutation, a refresh and an otto write can never be in flight at once.
//!
//! **No indicatif ticker.** `enable_steady_tick` spawns a thread inside
//! indicatif (`progress_bar.rs`) that locks and draws holding only indicatif's
//! locks, which is the hazard the design's Lifecycle section has to scope
//! honestly ("a panic there poisons the bar lock with no facade frame on the
//! stack"). It also would not work: a steady tick redraws a row whose message
//! nothing recomputed, so the duration in it would never advance. otto drives
//! the refresh itself, from one thread that goes through the facade like every
//! other writer, so every byte this module writes is written under otto's
//! ordering lock.

use std::{collections::VecDeque, sync::Arc, time::Duration};

use console::Term;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::debug;
use unicode_segmentation::UnicodeSegmentation;

use super::duration::format_task_duration;
use crate::executor::{clocks::TaskClock, colors::stream_task_label};

/// How often the refresh thread recomputes every row.
///
/// Under indicatif's own 20/s draw throttle on a stderr target, so a tick can
/// never be the thing that floods the terminal. Five a second keeps the seconds
/// field visibly live without the region being the busiest thing on the
/// machine, and bounds how long a torn-down region leaves its thread asleep.
pub const REFRESH_INTERVAL: Duration = Duration::from_millis(200);

/// How long a task must have been silent before its row says so.
///
/// Below this every row would carry an idle figure that changes faster than it
/// can be read, which is the noise the design's "duration names its subject"
/// goal is about. Above it, silence is the thing the reader is asking about.
const IDLE_THRESHOLD: Duration = Duration::from_secs(5);

/// Rows reserved above the region: the overflow row, the completion line about
/// to be printed, and the shell prompt the run returns to.
const RESERVED_ROWS: u16 = 6;

/// The fewest task rows the region will draw, however short the terminal is.
const MIN_ROWS: usize = 3;

/// The widest a name column gets before the rest of the row starts paying for
/// it. A long foreach subtask name overflows its column rather than pushing
/// every other row's duration off the screen.
const MAX_NAME_WIDTH: usize = 24;

/// One running task's row: its name and the clock the row reads.
///
/// It holds the `TaskClock` itself, not a copy of a start instant and not a
/// task name to look up in `task_start_times`. That is the design doc's Data
/// Model talking: a third independent start instant is how the row and the
/// completion line drift apart, and `task_start_times` is stamped before
/// dependency waits, which is a different number.
struct Row {
    /// The task's own name, as the scheduler spells it. What `finished` matches
    /// on.
    name: String,
    /// The name as it is safe to draw: [`sanitize`]d, so an ottofile cannot put
    /// an escape sequence into otto's own terminal region.
    label: String,
    clock: Arc<TaskClock>,
}

/// A row that is on the screen, with the bar drawing it.
struct Drawn {
    row: Row,
    bar: ProgressBar,
}

/// The live region.
pub struct Region {
    mp: MultiProgress,
    /// The rule between scrollback and the rows. Present only while at least
    /// one row is: a run with nothing in flight draws nothing at all.
    separator: Option<ProgressBar>,
    drawn: Vec<Drawn>,
    /// Running tasks past the row cap. They have no bar; the overflow row
    /// counts them.
    waiting: VecDeque<Row>,
    /// `... and N more running`. One row however many are hidden, which is the
    /// whole point of capping: indicatif drops the rows that do not fit, and
    /// without a cap the row it drops is this one.
    overflow: Option<ProgressBar>,
    /// Width of the name column, from the run's own task names.
    name_width: usize,
    /// Whether otto's own lines on stderr take colour. Read once: it is a
    /// property of the stream, not of a row.
    color: bool,
    /// Test-only row cap. Production reads the terminal every time it needs
    /// one, which is also how a resize is handled; a test cannot resize the
    /// harness's terminal and must not depend on what `console` invents for a
    /// captured one.
    #[cfg(test)]
    cap_override: Option<usize>,
}

impl Region {
    /// Build a region for a run over `task_names`.
    ///
    /// The `MultiProgress` is pinned to stderr explicitly rather than taken
    /// from `MultiProgress::new()`'s default, so the stream this draws on is
    /// stated in one place next to the style that must match it.
    pub fn new(task_names: &[String]) -> Self {
        let name_width = task_names
            .iter()
            .map(|name| sanitize(name).chars().count())
            .max()
            .unwrap_or(0)
            .min(MAX_NAME_WIDTH);
        debug!("Region::new: tasks={} name_width={name_width}", task_names.len());
        Self {
            mp: MultiProgress::with_draw_target(indicatif::ProgressDrawTarget::stderr()),
            separator: None,
            drawn: Vec::new(),
            waiting: VecDeque::new(),
            overflow: None,
            name_width,
            color: crate::executor::colors::stderr_takes_color(),
            #[cfg(test)]
            cap_override: None,
        }
    }

    /// A region with a fixed row cap, for the bounded-rows tests.
    #[cfg(test)]
    pub(super) fn with_cap(task_names: &[String], cap: usize) -> Self {
        Self {
            cap_override: Some(cap),
            ..Self::new(task_names)
        }
    }

    /// The labels currently ON the screen, top to bottom.
    #[cfg(test)]
    pub(super) fn row_labels(&self) -> Vec<String> {
        self.drawn.iter().map(|d| d.row.label.clone()).collect()
    }

    /// How many running tasks the overflow row is standing in for.
    #[cfg(test)]
    pub(super) fn waiting_len(&self) -> usize {
        self.waiting.len()
    }

    /// Whether the `... and N more running` row exists right now.
    #[cfg(test)]
    pub(super) fn has_overflow_row(&self) -> bool {
        self.overflow.is_some()
    }

    /// Whether anything is on the screen right now.
    ///
    /// The facade asks before suspending: a region with no rows has drawn no
    /// bytes, and clearing and redrawing nothing around every line of task
    /// output is pure cost.
    pub fn is_drawn(&self) -> bool {
        self.separator.is_some()
    }

    /// Hide the region around `f`, which writes to the terminal directly.
    ///
    /// `MultiProgress::suspend` clears, runs `f`, then force-redraws. That is
    /// the erase-before-write invariant the design doc wanted bound to one
    /// module.
    pub fn suspend<R>(&mut self, f: impl FnOnce() -> R) -> R {
        if !self.is_drawn() {
            return f();
        }
        self.mp.suspend(f)
    }

    /// A task started: give it a row, or count it against the overflow row.
    pub fn started(&mut self, name: &str, clock: Arc<TaskClock>) {
        if self.drawn.iter().any(|d| d.row.name == name) || self.waiting.iter().any(|r| r.name == name) {
            return;
        }
        let row = Row {
            name: name.to_string(),
            label: sanitize(name),
            clock,
        };
        if self.drawn.len() < self.cap() {
            self.attach(row);
        } else {
            self.waiting.push_back(row);
        }
        self.sync_overflow();
        self.redraw();
    }

    /// A task finished: take its row off the screen.
    ///
    /// `mp.remove` explicitly, per the design doc: indicatif's own `mark_zombie`
    /// reaps only the leading run of finished bars, so a task that finishes
    /// third leaves a dead row behind until the two ahead of it also finish.
    pub fn finished(&mut self, name: &str) {
        if let Some(index) = self.drawn.iter().position(|d| d.row.name == name) {
            let gone = self.drawn.remove(index);
            self.mp.remove(&gone.bar);
            if let Some(next) = self.waiting.pop_front() {
                self.attach(next);
            }
        } else {
            self.waiting.retain(|row| row.name != name);
        }
        if self.drawn.is_empty() {
            self.clear_all();
            return;
        }
        self.sync_overflow();
        self.redraw();
    }

    /// Recompute every row from its clock. Called on the refresh interval.
    pub fn refresh(&mut self) {
        if !self.is_drawn() {
            return;
        }
        self.redraw();
    }

    /// Take the region off the screen without forgetting it.
    ///
    /// `MultiProgress::clear`, not `set_draw_target(hidden())`: `disconnect` on
    /// a terminal draw target is a no-op, so swapping the target hides future
    /// draws and leaves the rows already on the screen exactly where they are.
    pub fn erase(&mut self) {
        let _ = self.mp.clear();
    }

    /// Put the region back after an erase, with every row recomputed.
    pub fn rearm(&mut self) {
        self.redraw();
    }

    /// How many task rows this terminal gets.
    ///
    /// `max(3, height - 6)`, clamped to `height - 1` so a terminal shorter than
    /// the floor cannot be asked for more rows than it has. Re-read per use
    /// rather than cached, which is also the SIGWINCH handling: a resized
    /// terminal is a new answer from the same call.
    fn cap(&self) -> usize {
        #[cfg(test)]
        if let Some(cap) = self.cap_override {
            return cap;
        }
        row_cap(Term::stderr().size().0)
    }

    /// Row width: the terminal minus one column, so a row that fills the line
    /// cannot wrap and desynchronize indicatif's line count from the screen.
    fn width(&self) -> usize {
        Term::stderr().size().1.saturating_sub(1).max(1) as usize
    }

    /// Give `row` a bar, ahead of the overflow row if there is one so that row
    /// stays last.
    fn attach(&mut self, row: Row) {
        self.ensure_separator();
        let bar = ProgressBar::new_spinner();
        let bar = match &self.overflow {
            Some(overflow) => self.mp.insert_before(overflow, bar),
            None => self.mp.add(bar),
        };
        // AFTER `add`, and with a template carrying no styled placeholder. On
        // indicatif 0.18.3 `ProgressDrawTarget::is_stderr()` answers `false`
        // for a `Multi` target, and `set_style` is its only consumer, so a
        // `{msg:.green}`-style template inside a `MultiProgress` would be
        // coloured by console's STDOUT decision while the region draws on
        // STDERR - the same defect otto fixed for its own status lines in
        // `47b4605`. otto colours the row text itself, from
        // `stderr_takes_color`, and hands indicatif nothing to decide.
        bar.set_style(row_style());
        self.drawn.push(Drawn { row, bar });
    }

    /// The separator rule, added on the first row and removed with the last.
    fn ensure_separator(&mut self) {
        if self.separator.is_some() {
            return;
        }
        let bar = self.mp.add(ProgressBar::new_spinner());
        bar.set_style(row_style());
        self.separator = Some(bar);
    }

    /// Add, drop, or renumber the `... and N more running` row.
    fn sync_overflow(&mut self) {
        match (self.waiting.len(), self.overflow.take()) {
            (0, Some(bar)) => self.mp.remove(&bar),
            (0, None) => {}
            (_, Some(bar)) => self.overflow = Some(bar),
            (_, None) => {
                let bar = self.mp.add(ProgressBar::new_spinner());
                bar.set_style(row_style());
                self.overflow = Some(bar);
            }
        }
    }

    /// Everything off the screen and out of the `MultiProgress`, leaving the
    /// region live and empty.
    fn clear_all(&mut self) {
        for gone in std::mem::take(&mut self.drawn) {
            self.mp.remove(&gone.bar);
        }
        if let Some(bar) = self.overflow.take() {
            self.mp.remove(&bar);
        }
        if let Some(bar) = self.separator.take() {
            self.mp.remove(&bar);
        }
        let _ = self.mp.clear();
    }

    /// Recompute every row's text and force one draw.
    ///
    /// A forced draw, because `set_message` alone is rate-limited and a removed
    /// bar does not redraw at all: without this a row that has just gone away
    /// stays on the screen until something else happens to draw.
    fn redraw(&mut self) {
        let width = self.width();
        if let Some(separator) = &self.separator {
            separator.set_message("-".repeat(width));
        }
        for drawn in &self.drawn {
            let text = row_text(&drawn.row, self.name_width, width, self.color);
            drawn.bar.set_message(text);
        }
        if let Some(overflow) = &self.overflow {
            overflow.set_message(truncate(&format!("... and {} more running", self.waiting.len()), width));
        }
        if let Some(separator) = &self.separator {
            separator.force_draw();
        }
    }
}

/// How many task rows fit a terminal `height` rows tall.
///
/// `max(3, height - 6)`, clamped to `height - 1` so a terminal shorter than the
/// floor cannot be asked for more rows than it has. The six are the separator,
/// the overflow row, the completion line about to print, and slack for the
/// shell prompt the run returns to.
fn row_cap(height: u16) -> usize {
    let room = height.saturating_sub(RESERVED_ROWS) as usize;
    room.max(MIN_ROWS).min(height.saturating_sub(1).max(1) as usize)
}

/// The style every row in the region wears.
///
/// `{wide_msg}`, not `{msg}`: `wide_msg` is the placeholder indicatif pads and
/// truncates against the terminal width, so a row longer than the screen is cut
/// rather than wrapped into a second line the region's own line count does not
/// know about. otto truncates first anyway; this is the backstop, not the
/// mechanism.
///
/// No styled placeholder, deliberately. See `Region::attach`.
fn row_style() -> ProgressStyle {
    ProgressStyle::with_template("{wide_msg}").expect("static template")
}

/// One task's row, as text.
fn row_text(row: &Row, name_width: usize, width: usize, color: bool) -> String {
    let idle = Duration::from_millis(row.clock.idle_ms());
    let silence = if idle >= IDLE_THRESHOLD {
        format!("   no output for {}", format_task_duration(idle))
    } else {
        String::new()
    };
    let plain = format!(
        "{:<name_width$}  running {}{silence}",
        row.label,
        format_task_duration(row.clock.elapsed()),
    );
    colorize_name(&truncate(&plain, width), &row.label, color)
}

/// Apply otto's per-task colour to the name at the head of a row, and only
/// there.
///
/// Skipped when truncation ate into the name: colouring half a name would put
/// an SGR reset in the middle of a cut grapheme run for no benefit.
fn colorize_name(text: &str, label: &str, color: bool) -> String {
    if !color || !text.starts_with(label) {
        return text.to_string();
    }
    // `no_prefix: true`: the region's rows carry bare names, not the `[task]`
    // form the output prefixes use.
    format!("{}{}", stream_task_label(label, true, true), &text[label.len()..])
}

/// Cut `text` to `width` display columns, on a grapheme boundary.
///
/// Graphemes rather than chars: a `char` boundary sits between a base character
/// and its combining mark, so cutting there puts half a cluster on the screen
/// and can leave the terminal's own column count disagreeing with otto's.
fn truncate(text: &str, width: usize) -> String {
    if console::measure_text_width(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for cluster in text.graphemes(true) {
        let cost = console::measure_text_width(cluster);
        if used + cost > width {
            break;
        }
        out.push_str(cluster);
        used += cost;
    }
    out
}

/// Make `text` safe to draw inside the region.
///
/// Task names come from an ottofile, and the region is the one otto surface
/// where a stray control byte does not just look wrong but moves otto's own
/// cursor. Tabs expand (the region is one line per row, and a tab's width
/// depends on where the row happens to sit); every other C0 byte, `DEL`, and
/// anything introduced by `ESC` is dropped, so the only SGR on a row is the SGR
/// otto put there.
fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\t' => out.push_str("    "),
            '\x1b' => {
                // Drop the introducer and the sequence behind it. A CSI ends at
                // its first byte in `@`..`~`; anything else is a two-character
                // escape, already consumed by dropping the next character.
                if let Some('[') = chars.next() {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
            }
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[path = "region_tests.rs"]
mod tests;
