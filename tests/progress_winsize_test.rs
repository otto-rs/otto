//! What the live region does on a terminal of a GIVEN height, and on one whose
//! height changes mid-run.
//!
//! Audit remediation for `docs/design/2026-09-16-live-progress-renderer.md`.
//! Every other pty test in the suite reaches otto through `script`, which
//! allocates a pty whose size the test cannot choose and cannot change: it
//! copies its own stdin's winsize, and under `cargo test` there is none, so
//! every one of those runs is 24x80. That is why nothing measured the row cap
//! against a short terminal and why `tests/progress_spike_pty_test.rs`'s
//! SIGWINCH fixture could claim to tell "handled" from "ignored" while never
//! changing a winsize.
//!
//! So this file owns the pty MASTER itself: `openpty` with the height it wants,
//! `TIOCSWINSZ` plus `SIGWINCH` to change it, and otto's three handles on the
//! slave so `TIOCGWINSZ` on stderr answers what the test set.
//!
//! Every fixture is self-terminating and bounded. A pty test that can hang is a
//! suite that can hang.

mod common;

use common::{OTTO_BIN, isolate};
use std::fs;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// How long any run here gets before the test gives up. Same bound and
/// reasoning as the other pty tests: the failure being guarded is an unbounded
/// hang, so the number only has to be finite and slack.
const PTY_TIMEOUT: Duration = Duration::from_secs(60);

/// The six tasks every fixture below runs, all at once and all for long enough
/// to be resized underneath. Six because the interesting heights cap the region
/// at one to three rows, so there is always something for the overflow row to
/// count.
const SIX_SLEEPERS: &str = r#"
otto:
  api: 1

tasks:
  t1:
    bash: sleep 6
  t2:
    bash: sleep 6
  t3:
    bash: sleep 6
  t4:
    bash: sleep 6
  t5:
    bash: sleep 6
  t6:
    bash: sleep 6
"#;

fn write_ottofile(dir: &Path, contents: &str) -> PathBuf {
    let path = dir.join("otto.yml");
    fs::write(&path, contents).expect("write the fixture ottofile");
    path
}

/// Strip SGR sequences so a row can be named by its plain text. Cursor control
/// survives: these fixtures are about which rows exist, not about where a
/// newline is.
fn strip_sgr(text: &str) -> String {
    regex::Regex::new(r"\x1b\[[0-9;]*m")
        .expect("static regex")
        .replace_all(text, "")
        .into_owned()
}

/// A pty whose size the test chooses, with otto on the other end of it.
struct SizedPty {
    master: OwnedFd,
    child: Child,
    seen: Arc<Mutex<Vec<u8>>>,
}

impl SizedPty {
    /// Start otto on a pty `rows` tall, with all three of its handles on the
    /// slave so `TIOCGWINSZ` answers `rows` for stderr.
    ///
    /// `CI` is removed and `TERM` pinned: `ProgressMode::resolve` is Quiet
    /// whenever `$CI` is set, which would make every assertion below vacuous on
    /// a CI runner rather than failing there.
    fn start(args: &[&str], home: &Path, cwd: &Path, rows: u16) -> Self {
        let mut master_fd = 0;
        let mut slave_fd = 0;
        let size = libc::winsize {
            ws_row: rows,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // Safe: both out-params are written on success, and the winsize is
        // read-only input.
        let rc = unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        };
        assert_eq!(rc, 0, "openpty failed: {}", std::io::Error::last_os_error());
        // Safe: `openpty` just handed both fds over and nothing else owns them.
        let master = unsafe { OwnedFd::from_raw_fd(master_fd) };
        let slave = unsafe { OwnedFd::from_raw_fd(slave_fd) };

        let mut cmd = Command::new(OTTO_BIN);
        cmd.args(args);
        isolate(&mut cmd, home);
        let child = cmd
            .current_dir(cwd)
            .env_remove("OTTOFILE")
            .env_remove("CI")
            .env_remove("NO_COLOR")
            .env("TERM", "xterm")
            .stdin(Stdio::from(slave.try_clone().expect("dup the pty slave")))
            .stdout(Stdio::from(slave.try_clone().expect("dup the pty slave")))
            .stderr(Stdio::from(slave.try_clone().expect("dup the pty slave")))
            .spawn()
            .expect("otto should spawn on the pty slave");
        // The parent's own handle on the slave goes here, so the master reads
        // end-of-file once otto exits instead of blocking forever.
        drop(slave);

        let seen = Arc::new(Mutex::new(Vec::new()));
        {
            let seen = Arc::clone(&seen);
            // Safe: this dup is the reader thread's alone, and it is closed
            // when the thread's `File` drops.
            let mut reader = unsafe { std::fs::File::from_raw_fd(libc::dup(master.as_raw_fd())) };
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    // A pty master reads `EIO` rather than 0 bytes once the
                    // last slave is closed, so an error IS the end here.
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => seen.lock().expect("output buffer").extend_from_slice(&buf[..n]),
                    }
                }
            });
        }

        Self { master, child, seen }
    }

    /// Resize the terminal under the running otto.
    ///
    /// `TIOCSWINSZ` is the part that matters - otto re-reads the winsize rather
    /// than handling a signal - and `SIGWINCH` goes with it so the fixture is
    /// the resize a terminal emulator actually performs.
    fn resize(&mut self, rows: u16) {
        let size = libc::winsize {
            ws_row: rows,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // Safe: an ioctl on the test's own master fd with a read-only argument.
        let rc = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) };
        assert_eq!(rc, 0, "TIOCSWINSZ failed: {}", std::io::Error::last_os_error());
        let pid = self.child.id() as libc::pid_t;
        // Safe: signalling a child this process spawned and has not reaped.
        unsafe { libc::kill(pid, libc::SIGWINCH) };
    }

    /// Everything the terminal has been sent so far, colours stripped.
    fn so_far(&self) -> String {
        let bytes = self.seen.lock().expect("output buffer").clone();
        strip_sgr(&String::from_utf8_lossy(&bytes))
    }

    /// Reap otto within [`PTY_TIMEOUT`] and return its exit code plus
    /// everything it wrote.
    fn finish(mut self) -> (i32, String) {
        let deadline = Instant::now() + PTY_TIMEOUT;
        let code = loop {
            match self.child.try_wait().expect("try_wait") {
                Some(status) => break status.code().unwrap_or(-1),
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
                None => {
                    let _ = self.child.kill();
                    panic!("otto did not exit within {PTY_TIMEOUT:?} on the pty");
                }
            }
        };
        // The reader thread ends on EIO once the last slave fd is gone; give it
        // the moment it needs to drain what is still buffered.
        thread::sleep(Duration::from_millis(200));
        (code, self.so_far())
    }
}

/// Every whole-second duration a row for `task` reported, in order. Same shape
/// as `tests/progress_region_test.rs`.
fn row_seconds(output: &str, task: &str) -> Vec<u64> {
    regex::Regex::new(&format!(r"{task}\s+running (\d+)s"))
        .expect("row regex")
        .captures_iter(output)
        .map(|c| c[1].parse().expect("digits"))
        .collect()
}

fn fixture(temp: &TempDir) -> (PathBuf, PathBuf) {
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    write_ottofile(temp.path(), SIX_SLEEPERS);
    (home, temp.path().to_path_buf())
}

const TASKS: [&str; 6] = ["t1", "t2", "t3", "t4", "t5", "t6"];

fn run_all(home: &Path, cwd: &Path, rows: u16) -> SizedPty {
    SizedPty::start(&["--jobs", "6", "t1", "t2", "t3", "t4", "t5", "t6"], home, cwd, rows)
}

// ---------------------------------------------------------------------------
// The row cap on a short terminal, with no resize involved.
// ---------------------------------------------------------------------------

/// Measured: six tasks on a three-row terminal drew ZERO `... and N more
/// running` rows, while the same fixture on an eight-row terminal drew them
/// correctly. `row_cap` counted neither the separator nor the overflow row
/// against the height, so heights 3 and 4 asked for more lines than the screen
/// had and indicatif dropped exactly the row the cap exists to protect.
#[test]
fn a_short_terminal_still_gets_the_overflow_row() {
    for rows in [3u16, 4, 8] {
        let temp = TempDir::new().unwrap();
        let (home, cwd) = fixture(&temp);
        let (code, out) = run_all(&home, &cwd, rows).finish();
        assert_eq!(code, 0, "the fixture must run clean at {rows} rows:\n{out}");
        assert!(
            out.contains("more running"),
            "six tasks on a {rows}-row terminal drew no overflow row:\n{out}"
        );
    }
}

// ---------------------------------------------------------------------------
// Resize: a terminal that actually changes size mid-run.
// ---------------------------------------------------------------------------

/// A shrink has to give rows back. Measured before this: shrinking 40 rows to 3
/// left all six rows attached and never produced an overflow row, because the
/// cap was read in `started` and nowhere else.
#[test]
fn shrinking_the_terminal_mid_run_hands_rows_to_the_overflow_row() {
    let temp = TempDir::new().unwrap();
    let (home, cwd) = fixture(&temp);
    let mut run = run_all(&home, &cwd, 40);

    thread::sleep(Duration::from_millis(1500));
    let before = run.so_far();
    assert!(
        !before.contains("more running"),
        "40 rows is room for all six tasks; the fixture is not starting from a full region:\n{before}"
    );
    run.resize(3);

    let (code, out) = run.finish();
    assert_eq!(code, 0, "the fixture must run clean:\n{out}");
    assert!(
        out.contains("more running"),
        "the region kept all six rows in a three-row terminal after the shrink:\n{out}"
    );
}

/// And a grow has to take them back. Measured before this: growing 8 rows to 40
/// promoted nobody - the overflow count only fell as tasks FINISHED.
///
/// Asserted through the duration in each row rather than through the row's mere
/// existence, because a hidden task does eventually get a row when a drawn one
/// finishes. At eight rows the cap is three, so at most three tasks can have a
/// row inside the first few seconds; after the grow, all six must.
#[test]
fn growing_the_terminal_mid_run_promotes_the_rows_it_now_has_room_for() {
    let temp = TempDir::new().unwrap();
    let (home, cwd) = fixture(&temp);
    let mut run = run_all(&home, &cwd, 8);

    thread::sleep(Duration::from_millis(1500));
    let before = run.so_far();
    assert!(
        before.contains("more running"),
        "eight rows must not be room for six tasks; the fixture is not starting from a capped region:\n{before}"
    );
    run.resize(40);

    let (code, out) = run.finish();
    assert_eq!(code, 0, "the fixture must run clean:\n{out}");
    let early: Vec<&str> = TASKS
        .iter()
        .copied()
        .filter(|task| row_seconds(&out, task).iter().any(|&s| s <= 4))
        .collect();
    assert_eq!(
        early.len(),
        TASKS.len(),
        "only {early:?} had a row while the run was still young; the grow promoted nobody:\n{out}"
    );
}
