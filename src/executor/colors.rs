use colored::{Color, Colorize};
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::hash::{Hash, Hasher};
use std::io::{self, IsTerminal};
use std::sync::{Mutex, OnceLock};

/// All 15 possible color combinations (bracket_color, text_color) where bracket ≠ text
/// This gives us 15 unique visual patterns before cycling
/// Ordered to ensure good bracket color distribution for the first several tasks
const COLOR_COMBINATIONS: [(Color, Color); 15] = [
    (Color::BrightRed, Color::BrightGreen),      // 0 - Red brackets
    (Color::BrightBlue, Color::BrightYellow),    // 1 - Blue brackets
    (Color::BrightGreen, Color::BrightBlue),     // 2 - Green brackets
    (Color::BrightYellow, Color::BrightCyan),    // 3 - Yellow brackets
    (Color::BrightCyan, Color::BrightMagenta),   // 4 - Cyan brackets
    (Color::BrightRed, Color::BrightBlue),       // 5 - Red brackets
    (Color::BrightBlue, Color::BrightCyan),      // 6 - Blue brackets
    (Color::BrightGreen, Color::BrightYellow),   // 7 - Green brackets
    (Color::BrightYellow, Color::BrightMagenta), // 8 - Yellow brackets
    (Color::BrightRed, Color::BrightYellow),     // 9 - Red brackets
    (Color::BrightBlue, Color::BrightMagenta),   // 10 - Blue brackets
    (Color::BrightGreen, Color::BrightCyan),     // 11 - Green brackets
    (Color::BrightRed, Color::BrightCyan),       // 12 - Red brackets
    (Color::BrightGreen, Color::BrightMagenta),  // 13 - Green brackets
    (Color::BrightRed, Color::BrightMagenta),    // 14 - Red brackets
];

/// Global task ordering context for consistent color assignment
static TASK_ORDER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

pub fn set_global_task_order(task_names: Vec<String>) {
    let mut sorted_names = task_names;
    sorted_names.sort();
    let task_order = TASK_ORDER.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut order) = task_order.lock() {
        *order = sorted_names;
    }
}

pub fn get_task_color_combination(task_name: &str) -> (Color, Color) {
    if let Some(task_order) = TASK_ORDER.get()
        && let Ok(order) = task_order.lock()
        && let Some(position) = order.iter().position(|name| name == task_name)
    {
        return COLOR_COMBINATIONS[position % COLOR_COMBINATIONS.len()];
    }

    // Fallback to hash-based assignment
    let mut hasher = DefaultHasher::new();
    task_name.hash(&mut hasher);
    let hash = hasher.finish();
    COLOR_COMBINATIONS[(hash as usize) % COLOR_COMBINATIONS.len()]
}

/// Get a consistent color for a task name using alphabetical ordering (legacy function for backwards compatibility)
pub fn get_task_color(task_name: &str) -> Color {
    // Return just the bracket color for backwards compatibility
    get_task_color_combination(task_name).0
}

/// Format a task name with its assigned color
pub fn colorize_task_name(task_name: &str) -> String {
    if colored::control::SHOULD_COLORIZE.should_colorize() {
        task_name.color(get_task_color(task_name)).to_string()
    } else {
        task_name.to_string()
    }
}

/// Format a task prefix (e.g., "\[task_name\]") with two-color system: colored brackets + colored text
pub fn colorize_task_prefix(task_name: &str) -> String {
    if colored::control::SHOULD_COLORIZE.should_colorize() {
        let (bracket_color, text_color) = get_task_color_combination(task_name);
        format!(
            "{}{}{}",
            "[".color(bracket_color),
            task_name.color(text_color),
            "]".color(bracket_color)
        )
    } else {
        format!("[{task_name}]")
    }
}

/// What a line otto writes about a task leads with: `[task]`, or a bare `task`
/// under `--no-prefix`.
///
/// One function for both the scheduler's status lines and the heartbeat, so the
/// two cannot drift into labelling the same task differently.
pub fn task_label(task_name: &str, no_prefix: bool) -> String {
    if no_prefix {
        colorize_task_name(task_name)
    } else {
        colorize_task_prefix(task_name)
    }
}

/// The same label with no colour, whatever `SHOULD_COLORIZE` says.
///
/// For a line going to a stderr that takes no colour. `colored` derives
/// `SHOULD_COLORIZE` from `stdout().is_terminal()`
/// (`colored-3.1.1/src/control.rs:108`), so [`task_label`] applies a decision about
/// stdout to whatever stream it is printed on - which is how `otto task 2>log`
/// with stdout on a terminal used to write a coloured label into `log`. A
/// caller writing to stderr picks between the two through
/// [`stream_task_label`].
pub fn plain_task_label(task_name: &str, no_prefix: bool) -> String {
    if no_prefix { task_name.to_string() } else { format!("[{task_name}]") }
}

/// The stderr colour decision as a function of its two inputs, so every
/// combination is testable without mutating a process-shared environment.
///
/// `force` is `CLICOLOR_FORCE`'s value when it is set and valid UTF-8. The
/// `!= "0"` test and the treatment of a non-UTF-8 value as unset are
/// `colored`'s own `normalize_env` (`colored-3.1.1/src/control.rs:144`), copied
/// rather than reinterpreted: otto and `colored` must not disagree about what
/// `CLICOLOR_FORCE=yes` means. Note `CLICOLOR_FORCE=0` does not suppress
/// colour on a terminal, in `colored` either - it resolves to no override and
/// falls through to the tty check.
fn stderr_takes_color_from(is_terminal: bool, force: Option<&str>) -> bool {
    is_terminal || force.is_some_and(|value| value != "0")
}

/// Whether a line otto writes on its own stderr takes colour. The one answer in
/// the binary.
///
/// True when stderr is a terminal, and also under `CLICOLOR_FORCE`: `colored`
/// documents that variable as its highest-priority override, ahead of
/// `NO_COLOR` and the tty check (`from_env`,
/// `colored-3.1.1/src/control.rs:102`), and it is the knob a user reaches for
/// to pipe colour into a file. A predicate that asked only about the terminal
/// vetoed it.
///
/// Read once and cached on purpose: nothing about a run can change whether
/// stderr is a terminal or what the environment said at startup, and npm
/// shipped a progress predicate evaluated twice whose two readings diverged
/// (commit `5b858c6`).
///
/// Not by itself the whole decision to colour: it answers only the half
/// `colored` gets wrong, which stream the bytes land on. `NO_COLOR`, `CLICOLOR`
/// and the stdout tty read stay with `colored`, reached through [`task_label`],
/// as does `CLICOLOR_FORCE` for stdout.
pub fn stderr_takes_color() -> bool {
    static STDERR_TAKES_COLOR: OnceLock<bool> = OnceLock::new();
    *STDERR_TAKES_COLOR.get_or_init(|| {
        let force = env::var("CLICOLOR_FORCE").ok();
        stderr_takes_color_from(io::stderr().is_terminal(), force.as_deref())
    })
}

/// The label for a line about to be written to a stream that takes colour
/// (`takes_color`) or does not: [`task_label`] or [`plain_task_label`].
///
/// The decision comes in as a parameter because the caller is the one that
/// knows which of otto's two streams it is writing on, and because that keeps
/// every label site pure and unit-testable without a real terminal. stderr
/// sites pass [`stderr_takes_color`]; stdout sites pass `true` and let
/// `colored` have the last word.
pub fn stream_task_label(task_name: &str, no_prefix: bool, takes_color: bool) -> String {
    if takes_color {
        task_label(task_name, no_prefix)
    } else {
        plain_task_label(task_name, no_prefix)
    }
}

#[path = "colors_tests.rs"]
mod tests;
