//! otto's own two handles, wired to two different places.
//!
//! Audit remediation (round 2, N1 and N2) for
//! `docs/design/2026-09-16-live-progress-renderer.md`. Two defects with one
//! cause: the facade asked a question about ONE handle and applied the answer
//! to the terminal.
//!
//! N1, one terminal through two device files. `otto task >/dev/tty` on the
//! terminal whose pts slave is otto's stderr puts both handles on one cursor,
//! but the two fds do not answer the same `fstat` triple: `/dev/tty` is the
//! `(5,0)` character device and the slave is a `devpts` inode. Comparing
//! triples and nothing else called them separate destinations, kept a separate
//! line state per stream, and put otto's completion line on the end of a task's
//! unterminated last line - the defect the per-stream state was added to fix,
//! returning through the back door.
//!
//! N2, one terminal and one capture. The defensive `\x1b[0m` that closes a
//! `tty:` child's SGR state was gated on stderr being a terminal and written to
//! stderr, but a `tty:` child inherits BOTH handles, so `otto owner 2>log` left
//! the terminal stdout red for every line of the rest of the run.
//!
//! `--progress never` in the N1 fixtures on purpose: with a live region
//! installed, the region's own `ensure_line_start` emits the missing newline as
//! a side effect of drawing, which masks the coupling question entirely. The
//! quiet path is where otto's line state is the only thing that knows where the
//! cursor is.
//!
//! This file owns the pty MASTER, and for the `/dev/tty` fixtures the child's
//! controlling terminal too: the child `setsid`s, claims the slave with
//! `TIOCSCTTY`, and only then opens `/dev/tty`, so `/dev/tty` resolves to this
//! test's pty rather than to whatever the test runner was started from (under
//! `cargo test` there is usually nothing there at all).

mod common;

use common::{OTTO_BIN, isolate};
use std::ffi::CString;
use std::fs;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// How long a run here gets before the test gives up. Same bound and reasoning
/// as every other pty test in the suite: the failure being guarded is an
/// unbounded hang, so the number only has to be finite and slack.
const PTY_TIMEOUT: Duration = Duration::from_secs(60);

/// A task whose last chunk has no trailing newline, on stderr. otto's own
/// completion line goes to stdout, so this fixture is exactly the cross-stream
/// question: stderr left the cursor mid-line and stdout has to know.
const UNTERMINATED_STDERR: &str = r#"
otto:
  api: 1

tasks:
  t:
    bash: printf ERR >&2
"#;

/// A `tty:` child that exits with red still active, on its stdout, and a task
/// after it whose output shows whether the colour is still on. Nothing in otto's
/// own output turns it off unless otto says so, which is what `NO_COLOR` in the
/// fixture below guarantees.
///
/// `owner`'s `after: [later]` is otto's spelling of "run `later` after me"
/// (compare `cov: {after: [cov-report]}` in this repo's own `.otto.yml`), and
/// the admission gate would defer the `tty:` task past `later` otherwise.
const SGR_LEAK_ON_STDOUT: &str = r#"
otto:
  api: 1

tasks:
  owner:
    tty: true
    after: [later]
    bash: |
      printf '\033[31mOWNER\n'
  later:
    bash: echo LATER
"#;

/// Where otto's two handles go. Neither of these is exotic: the first two are
/// `otto >/dev/tty` and `otto 2>/dev/tty` on the terminal otto is already
/// running on, and the third is `otto 2>log`.
enum Wiring {
    StdoutViaDevTty,
    StderrViaDevTty,
    StderrToFile(PathBuf),
}

/// otto on a pty, with its two output handles wired per [`Wiring`].
struct SplitHandles {
    child: Child,
    seen: Arc<Mutex<Vec<u8>>>,
    /// Kept alive so the reader thread's dup does not see the last slave close
    /// before otto has written anything.
    _master: OwnedFd,
}

impl SplitHandles {
    fn start(args: &[&str], home: &Path, cwd: &Path, wiring: Wiring) -> Self {
        let mut master_fd = 0;
        let mut slave_fd = 0;
        // Safe: both out-params are written on success and nothing else is
        // passed in.
        let rc = unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                // `*mut`, not `*const`: glibc declares these two `const` and
                // Apple's libc does not, so a null `*const` builds on Linux
                // and fails E0308 on macOS. A null `*mut` satisfies both.
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0, "openpty failed: {}", std::io::Error::last_os_error());
        // Safe: `openpty` just handed both fds over and nothing else owns them.
        let master = unsafe { OwnedFd::from_raw_fd(master_fd) };
        let slave = unsafe { OwnedFd::from_raw_fd(slave_fd) };

        let mut cmd = Command::new(OTTO_BIN);
        cmd.args(args);
        isolate(&mut cmd, home);
        // `CI` is removed and `TERM` pinned for the same reason every other pty
        // test here does it: `ProgressMode::resolve` is Quiet whenever `$CI` is
        // set, which would make a live-path assertion vacuous on a CI runner
        // rather than failing there. `NO_COLOR` is the N2 fixture itself - with
        // colour on, otto's own coloured label carries a reset of its own and
        // masks the bleed.
        cmd.current_dir(cwd)
            .env_remove("OTTOFILE")
            .env_remove("CI")
            .env("NO_COLOR", "1")
            .env("TERM", "xterm")
            .stdin(Stdio::from(slave.try_clone().expect("dup the pty slave")))
            .stdout(Stdio::from(slave.try_clone().expect("dup the pty slave")))
            .stderr(Stdio::from(slave.try_clone().expect("dup the pty slave")));

        match wiring {
            Wiring::StderrToFile(path) => {
                let log = fs::File::create(&path).expect("open the stderr capture");
                cmd.stderr(Stdio::from(log));
            }
            Wiring::StdoutViaDevTty => reopen_through_dev_tty(&mut cmd, libc::STDOUT_FILENO),
            Wiring::StderrViaDevTty => reopen_through_dev_tty(&mut cmd, libc::STDERR_FILENO),
        }

        let child = cmd.spawn().expect("otto should spawn on the pty slave");
        // The parent's own handle on the slave goes here, so the master reads
        // end-of-file once otto exits instead of blocking forever.
        drop(slave);

        let seen = Arc::new(Mutex::new(Vec::new()));
        {
            let seen = Arc::clone(&seen);
            // Safe: this dup is the reader thread's alone, and it is closed when
            // the thread's `File` drops.
            let mut reader = unsafe { fs::File::from_raw_fd(libc::dup(master.as_raw_fd())) };
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    // A pty master reads `EIO` rather than 0 bytes once the last
                    // slave is closed, so an error IS the end here.
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => seen.lock().expect("output buffer").extend_from_slice(&buf[..n]),
                    }
                }
            });
        }

        Self {
            child,
            seen,
            _master: master,
        }
    }

    /// Reap otto within [`PTY_TIMEOUT`] and return its exit code plus every
    /// byte the terminal was sent, with the pty's CRLF folded back to `\n`.
    /// Colours survive: one fixture here is about an escape sequence.
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
        let bytes = self.seen.lock().expect("output buffer").clone();
        (code, String::from_utf8_lossy(&bytes).replace('\r', ""))
    }
}

/// Give the child its own session with the pty slave as its controlling
/// terminal, then re-open that terminal through `/dev/tty` onto `target`.
///
/// Both halves are needed: `/dev/tty` is the calling process's controlling
/// terminal, and a `cargo test` child has none until it claims one.
fn reopen_through_dev_tty(cmd: &mut Command, target: i32) {
    let dev_tty = CString::new("/dev/tty").expect("static path");
    // Safe: the closure runs between fork and exec in a single-threaded child
    // and calls nothing but async-signal-safe libc entry points. The `CString`
    // is allocated above, before the fork.
    unsafe {
        cmd.pre_exec(move || {
            // A new session with no controlling terminal, so the slave can
            // become one. Cannot fail here: a freshly forked child is never
            // already a process-group leader.
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            // fd 0 is the slave: `Command` dup2'd the configured stdio before
            // this closure runs.
            // `ioctl`'s request is `c_ulong` on both platforms, but Apple
            // types the TIOCSCTTY constant `c_uint` while glibc already types
            // it `c_ulong`. So the conversion is REQUIRED on macOS (E0308
            // without it) and redundant on Linux, where clippy's
            // `useless_conversion` rejects it. One spelling has to build on
            // both, so the lint is allowed here rather than the code being
            // split behind a `cfg`.
            #[allow(clippy::useless_conversion)]
            let request = libc::c_ulong::from(libc::TIOCSCTTY);
            // fd 0 is the slave: `Command` dup2'd the configured stdio before
            // this closure runs.
            if libc::ioctl(libc::STDIN_FILENO, request, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let fd = libc::open(dev_tty.as_ptr(), libc::O_WRONLY);
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::dup2(fd, target) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(fd);
            Ok(())
        });
    }
}

fn fixture(temp: &TempDir, ottofile: &str) -> (PathBuf, PathBuf) {
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    fs::write(temp.path().join("otto.yml"), ottofile).expect("ottofile");
    (home, temp.path().to_path_buf())
}

// ---------------------------------------------------------------------------
// N1: one terminal, two device files.
// ---------------------------------------------------------------------------

/// What both directions assert: the run worked, the fixture actually reached
/// the terminal, and otto's own line started at column zero.
fn assert_one_cursor(text: &str, code: i32) {
    assert_eq!(code, 0, "the fixture must run clean:\n{text:?}");
    assert!(
        text.contains("ERR"),
        "the task's unterminated chunk never reached the terminal, so nothing is being measured:\n{text:?}"
    );
    assert!(
        text.contains("finished successfully"),
        "otto wrote no completion line, so nothing is being measured:\n{text:?}"
    );
    assert!(
        text.contains("ERR\n"),
        "otto's own line landed on the end of the task's unterminated one: \
         two device files, one terminal, one cursor\n{text:?}"
    );
}

#[test]
fn stdout_on_dev_tty_shares_the_cursor_with_stderr_on_the_slave() {
    let temp = TempDir::new().unwrap();
    let (home, cwd) = fixture(&temp, UNTERMINATED_STDERR);
    let run = SplitHandles::start(&["--progress", "never", "t"], &home, &cwd, Wiring::StdoutViaDevTty);
    let (code, text) = run.finish();
    assert_one_cursor(&text, code);
}

#[test]
fn stderr_on_dev_tty_shares_the_cursor_with_stdout_on_the_slave() {
    let temp = TempDir::new().unwrap();
    let (home, cwd) = fixture(&temp, UNTERMINATED_STDERR);
    let run = SplitHandles::start(&["--progress", "never", "t"], &home, &cwd, Wiring::StderrViaDevTty);
    let (code, text) = run.finish();
    assert_one_cursor(&text, code);
}

// ---------------------------------------------------------------------------
// N2: one terminal, one capture.
// ---------------------------------------------------------------------------

/// `otto owner 2>log`: the terminal the `tty:` child dirtied is stdout, and the
/// reset has to follow the terminal rather than the stream the region draws on.
///
/// The sibling fixture with BOTH handles on the terminal lives in
/// `tests/tty_task_test.rs`; this one is the half that was missing.
#[test]
fn the_reset_follows_the_terminal_when_stderr_is_captured() {
    let temp = TempDir::new().unwrap();
    let (home, cwd) = fixture(&temp, SGR_LEAK_ON_STDOUT);
    let log = temp.path().join("stderr.log");
    let run = SplitHandles::start(&["owner", "later"], &home, &cwd, Wiring::StderrToFile(log));
    let (code, text) = run.finish();

    assert_eq!(code, 0, "the fixture must run clean:\n{text:?}");
    let owner = text
        .find("OWNER")
        .unwrap_or_else(|| panic!("the tty child never reached the terminal, so nothing is being measured:\n{text:?}"));
    let completion = text
        .find("[owner] finished successfully")
        .unwrap_or_else(|| panic!("otto wrote no completion line, so nothing is being measured:\n{text:?}"));
    let reset = text[owner..].find("\x1b[0m").map(|at| at + owner).unwrap_or_else(|| {
        panic!("otto never closed the child's SGR state on the handle that IS a terminal:\n{text:?}")
    });
    let later = text
        .find("[later] LATER")
        .unwrap_or_else(|| panic!("the task after the tty child never ran, so nothing is being measured:\n{text:?}"));
    assert!(
        reset < completion && reset < later,
        "the reset landed AFTER otto's own lines, which are already rendered red:\n{text:?}"
    );
}
