//! The CLI-level twin of `tests/redirected_stderr_color_test.rs`: four messages
//! that otto prints on its own stderr before (or instead of) running anything,
//! and that colour their own words rather than a `[task]` label.
//!
//! - `ottofile_not_found_message` (`src/cli/parser.rs`), reached as an `eyre`
//!   report when a task is named in a tree with no ottofile;
//! - `ottofile_parse_error_message` (`src/cli/parser.rs`), reached by a
//!   malformed ottofile plus `otto --help`;
//! - `otto History` and `otto Stats` with no state database.
//!
//! Each called `colored`'s `.red()`/`.yellow()` directly, so each inherited
//! `SHOULD_COLORIZE` - derived from `stdout().is_terminal()`
//! (`colored-3.1.1/src/control.rs:108`) - and painted a redirected stderr
//! whenever stdout happened to be a terminal.
//!
//! Every assertion here comes in a pair, because "no escape bytes" is also what
//! a binary with all colour deleted produces: each site is checked plain in the
//! redirect AND coloured under `CLICOLOR_FORCE=1` and on a pty stderr.
//! [`the_help_epilogue_on_stdout_is_still_coloured`] pins the third direction,
//! since the not-found message is the one string otto prints on both streams.
//!
//! `NO_COLOR`, `CLICOLOR` and `CLICOLOR_FORCE` are removed from every child and
//! set back per test, for the reason `tests/redirected_stderr_color_test.rs`
//! gives: any of them exported in the developer's shell would decide these runs
//! instead of the test doing it, and the zero-escape assertions would then hold
//! against a binary with the fix ripped out.

mod common;

use common::{OTTO_BIN, isolate, pty_cmd};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// The ESC byte, counted by value. `tr -cd '\x1b'` cannot do this: GNU `tr` has
/// no `\x` escape and silently counts the literal characters `x`, `1` and `b`.
const ESC: u8 = 0x1b;

fn count_esc(bytes: &[u8]) -> usize {
    bytes.iter().filter(|b| **b == ESC).count()
}

/// An ottofile that parses as YAML and fails the typed parse: `before:` wants a
/// sequence and gets a map, so serde reports the field path plus line/column.
const UNPARSEABLE_OTTOFILE: &str =
    "otto:\n  api: 1\n\ntasks:\n  up:\n    before:\n      key: value\n    action: echo up\n";

/// The four sites, each as the cwd and argv that reaches it.
#[derive(Clone, Copy)]
enum Site {
    NotFound,
    ParseError,
    History,
    Stats,
}

impl Site {
    fn name(self) -> &'static str {
        match self {
            Site::NotFound => "not-found",
            Site::ParseError => "parse-error",
            Site::History => "history",
            Site::Stats => "stats",
        }
    }

    fn argv(self) -> &'static str {
        match self {
            Site::NotFound => "some-task",
            Site::ParseError => "--help",
            Site::History => "History",
            Site::Stats => "Stats",
        }
    }

    /// A phrase from the message, so a run that never reached the site fails on
    /// the words rather than passing on an empty redirect.
    fn phrase(self) -> &'static str {
        match self {
            Site::NotFound => "ERROR: No ottofile found in this directory or any parent directory!",
            Site::ParseError => "ERROR: failed to parse ottofile:",
            Site::History => "No history database found. Run otto to create it.",
            Site::Stats => "No statistics database found. Run otto to create it.",
        }
    }

    /// How many escape bytes the coloured form carries: two per styled span.
    /// The not-found message styles its headline plus each of the six ottofile
    /// names it lists.
    fn coloured_escapes(self) -> usize {
        match self {
            Site::NotFound => 14,
            Site::ParseError => 4,
            Site::History | Site::Stats => 2,
        }
    }
}

/// A fixture for `site`: the directory to run in, and the `OTTO_HOME` to run
/// with.
///
/// `History` and `Stats` print their message only when `StateManager::try_new()`
/// returns `None`, so their home is a path that cannot be created: its parent
/// is a regular file. The other two need a working home and a particular tree -
/// a fresh `TempDir` has no ottofile in it or any of its equally fresh
/// ancestors, which is the not-found state.
struct Fixture {
    _dir: TempDir,
    cwd: PathBuf,
    home: PathBuf,
}

fn fixture(site: Site) -> Fixture {
    let dir = TempDir::new().expect("tempdir");
    let cwd = dir.path().join("project");
    fs::create_dir_all(&cwd).expect("create project dir");

    let home = match site {
        Site::History | Site::Stats => {
            let blocker = dir.path().join("not-a-directory");
            fs::write(&blocker, b"").expect("write the blocker file");
            blocker.join("otto-home")
        }
        _ => {
            let home = dir.path().join("otto-home");
            fs::create_dir_all(&home).expect("create otto home");
            home
        }
    };

    if matches!(site, Site::ParseError) {
        fs::write(cwd.join(".otto.yml"), UNPARSEABLE_OTTOFILE).expect("write the unparseable ottofile");
    }

    Fixture { _dir: dir, cwd, home }
}

/// Single-quote one argv element for the `sh -c` string `script` runs.
fn quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// Run otto with `argv` under a real pty, with `suffix` appended for the
/// caller's redirection, and hand back the pty's stdout.
fn run(fixture: &Fixture, argv: &str, suffix: &str, colour_env: &[(&str, &str)]) -> Vec<u8> {
    let command = format!("{} {argv} {suffix}", quote(OTTO_BIN));
    let mut cmd = pty_cmd(&["sh", "-c", &command]);
    isolate(&mut cmd, &fixture.home);
    cmd.current_dir(&fixture.cwd)
        .env_remove("OTTOFILE")
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE");
    for (name, value) in colour_env {
        cmd.env(name, value);
    }
    cmd.output().expect("run otto under script").stdout
}

/// Run `argv` with stdout on the pty and stderr in a file, and hand back both.
fn split_streams(fixture: &Fixture, argv: &str, colour_env: &[(&str, &str)]) -> (Vec<u8>, Vec<u8>) {
    let err_path = fixture.cwd.join("stderr.txt");
    let suffix = format!("2> {}", quote(err_path.to_str().expect("utf-8 path")));
    let stdout = run(fixture, argv, &suffix, colour_env);
    let err = fs::read(&err_path).expect("read the stderr redirect");
    (stdout, err)
}

/// The redirect carries the message and not one escape byte. stdout is on a
/// pty for every one of these runs, which is the condition that used to leak:
/// `colored` said yes about stdout and the answer rode onto stderr.
fn assert_plain_in_the_redirect(site: Site) {
    let fixture = fixture(site);
    let (_, err) = split_streams(&fixture, site.argv(), &[]);
    let text = String::from_utf8_lossy(&err);

    assert!(
        text.contains(site.phrase()),
        "the {} run must have reached its message: {text:?}",
        site.name()
    );
    assert_eq!(
        count_esc(&err),
        0,
        "the {} message must carry no colour into a redirected stderr: {text:?}",
        site.name()
    );
}

/// `CLICOLOR_FORCE` is `colored`'s highest-priority override (`from_env`,
/// `colored-3.1.1/src/control.rs:102`) and the knob for piping colour into a
/// file, so the escapes must come back in the redirect under it.
fn assert_forced_colour_in_the_redirect(site: Site) {
    let fixture = fixture(site);
    let (_, err) = split_streams(&fixture, site.argv(), &[("CLICOLOR_FORCE", "1")]);
    let text = String::from_utf8_lossy(&err);

    assert!(
        text.contains(site.phrase()),
        "the {} run must have reached its message: {text:?}",
        site.name()
    );
    assert_eq!(
        count_esc(&err),
        site.coloured_escapes(),
        "under CLICOLOR_FORCE the {} message keeps every one of its coloured spans: {text:?}",
        site.name()
    );
}

/// With stderr left on the pty the message is coloured exactly as before the
/// fix, which is why the predicate asks about stderr instead of suppressing
/// colour on it.
fn assert_colour_on_a_pty_stderr(site: Site) {
    let fixture = fixture(site);
    let combined = run(&fixture, site.argv(), "2>&1", &[]);
    let text = String::from_utf8_lossy(&combined);

    assert!(
        text.contains(site.phrase()),
        "the {} run must have reached its message: {text:?}",
        site.name()
    );
    assert!(
        count_esc(&combined) >= site.coloured_escapes(),
        "a {} message on a terminal stderr keeps its colour: {text:?}",
        site.name()
    );
}

#[test]
fn the_not_found_message_is_plain_in_a_redirected_stderr() {
    assert_plain_in_the_redirect(Site::NotFound);
}

#[test]
fn the_parse_error_message_is_plain_in_a_redirected_stderr() {
    assert_plain_in_the_redirect(Site::ParseError);
}

#[test]
fn the_missing_history_db_message_is_plain_in_a_redirected_stderr() {
    assert_plain_in_the_redirect(Site::History);
}

#[test]
fn the_missing_stats_db_message_is_plain_in_a_redirected_stderr() {
    assert_plain_in_the_redirect(Site::Stats);
}

#[test]
fn clicolor_force_colours_the_not_found_message_in_a_redirect() {
    assert_forced_colour_in_the_redirect(Site::NotFound);
}

#[test]
fn clicolor_force_colours_the_parse_error_message_in_a_redirect() {
    assert_forced_colour_in_the_redirect(Site::ParseError);
}

#[test]
fn clicolor_force_colours_the_missing_history_db_message_in_a_redirect() {
    assert_forced_colour_in_the_redirect(Site::History);
}

#[test]
fn clicolor_force_colours_the_missing_stats_db_message_in_a_redirect() {
    assert_forced_colour_in_the_redirect(Site::Stats);
}

#[test]
fn a_pty_stderr_still_colours_the_not_found_message() {
    assert_colour_on_a_pty_stderr(Site::NotFound);
}

#[test]
fn a_pty_stderr_still_colours_the_parse_error_message() {
    assert_colour_on_a_pty_stderr(Site::ParseError);
}

#[test]
fn a_pty_stderr_still_colours_the_missing_history_db_message() {
    assert_colour_on_a_pty_stderr(Site::History);
}

#[test]
fn a_pty_stderr_still_colours_the_missing_stats_db_message() {
    assert_colour_on_a_pty_stderr(Site::Stats);
}

/// The `!= "0"` half of `colored`'s `normalize_env`
/// (`colored-3.1.1/src/control.rs:144`): the variable being *set* is not the
/// rule, so `CLICOLOR_FORCE=0` forces nothing and the redirect stays plain.
#[test]
fn a_zero_clicolor_force_leaves_the_redirect_plain() {
    for site in [Site::NotFound, Site::ParseError, Site::History, Site::Stats] {
        let fixture = fixture(site);
        let (_, err) = split_streams(&fixture, site.argv(), &[("CLICOLOR_FORCE", "0")]);
        let text = String::from_utf8_lossy(&err);

        assert!(
            text.contains(site.phrase()),
            "{} did not reach its message: {text:?}",
            site.name()
        );
        assert_eq!(
            count_esc(&err),
            0,
            "CLICOLOR_FORCE=0 forces nothing, so the {} redirect stays plain: {text:?}",
            site.name()
        );
    }
}

/// The third direction for the not-found message: it is also clap's help
/// epilogue on stdout, where the fix must change nothing. A `takes_color`
/// wired to `stderr_takes_color()` at every call site would strip this too.
#[test]
fn the_help_epilogue_on_stdout_is_still_coloured() {
    // `--help` in the same ottofile-less tree the not-found assertions use, so
    // the epilogue is the same string they see plain on stderr.
    let fixture = fixture(Site::NotFound);
    let (stdout, err) = split_streams(&fixture, "--help", &[]);
    let text = String::from_utf8_lossy(&stdout);

    assert!(
        text.contains("ERROR: No ottofile found in this directory or any parent directory!"),
        "the epilogue must be on stdout: {text:?}"
    );
    assert!(
        count_esc(&stdout) > 0,
        "the help epilogue on a terminal stdout must keep its colour: {text:?}"
    );
    assert_eq!(
        count_esc(&err),
        0,
        "nothing coloured belongs in the redirect on this path: {:?}",
        String::from_utf8_lossy(&err)
    );
}

/// The premise every zero-escape assertion above rests on: `script` really
/// allocated a pty, so stdout was a terminal and `colored` really did say yes.
/// Without a pty, otto is right to print plain and the assertions prove nothing.
#[test]
fn the_pty_fixture_puts_a_terminal_on_stdout() {
    let fixture = fixture(Site::ParseError);
    let (stdout, _) = split_streams(&fixture, Site::ParseError.argv(), &[]);

    assert!(
        stdout.contains(&b'\r'),
        "script must have allocated a pty (a pty ends lines with CRLF); got {:?}",
        String::from_utf8_lossy(&stdout)
    );
    assert!(
        count_esc(&stdout) > 0,
        "clap colours its help on a terminal stdout, so the absence of colour on stderr means something; got {:?}",
        String::from_utf8_lossy(&stdout)
    );
}

/// `fixture` really does block the state database for the two no-database
/// sites: if `StateManager::try_new()` ever succeeded there, `History` would
/// print a table and the four assertions about its message would be vacuous.
#[test]
fn the_blocked_home_really_denies_the_state_database() {
    let fixture = fixture(Site::History);
    let parent = fixture.home.parent().expect("the home has a parent");

    assert!(
        parent.is_file(),
        "the blocker must be a regular file, or the home is creatable: {}",
        parent.display()
    );
    assert!(
        !Path::new(&fixture.home).exists(),
        "the home must not exist: {}",
        fixture.home.display()
    );
}
