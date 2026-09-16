#![cfg(test)]

use super::*;
use crate::executor::heartbeat::TaskClocks;

#[tokio::test]
async fn test_output_processing() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output_dir = PathBuf::from(temp_dir.path());

    let streams = TaskStreams::new("test_task", &output_dir).await.unwrap();

    let test_output = "line 1\nline 2\nline 3\n";
    let mut rx = streams.output_tx.subscribe();

    // Process the output
    let mut cursor = std::io::Cursor::new(test_output);
    streams
        .process_output(
            "test_task".to_string(),
            OutputType::Stdout,
            &mut cursor,
            false,
            false,
            TaskClocks::default().start("test_task"),
        )
        .await
        .unwrap();

    let contents = streams.read_output(OutputType::Stdout).await.unwrap();
    assert_eq!(contents.len(), 3);
    assert_eq!(contents[0], "line 1");

    let received = rx.try_recv().unwrap();
    assert_eq!(received.task_name, "test_task");
    assert_eq!(received.content, "line 1\n");
}

/// A non-UTF-8 byte mid-stream used to end the drain: `read_line`'s
/// InvalidData error was read as EOF, so the terminal and `stdout.log` both
/// stopped at line 1 and the task still reported success.
#[tokio::test]
async fn a_non_utf8_byte_does_not_truncate_the_log() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output_dir = PathBuf::from(temp_dir.path());

    let streams = TaskStreams::new("test_task", &output_dir).await.unwrap();

    let mut test_output: Vec<u8> = Vec::new();
    test_output.extend_from_slice(b"line1\n");
    test_output.extend_from_slice(b"\xff\xfe bad\n");
    test_output.extend_from_slice(b"line3\nline4\nline5\n");

    let mut cursor = std::io::Cursor::new(test_output);
    streams
        .process_output(
            "test_task".to_string(),
            OutputType::Stdout,
            &mut cursor,
            true,
            false,
            TaskClocks::default().start("test_task"),
        )
        .await
        .unwrap();

    let contents = streams.read_output(OutputType::Stdout).await.unwrap();
    assert_eq!(contents.len(), 5, "every line must survive the bad byte: {contents:?}");
    assert_eq!(contents[0], "line1");
    assert_eq!(contents[4], "line5");
}

#[tokio::test]
async fn test_multiple_streams() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output_dir = PathBuf::from(temp_dir.path());

    let streams = TaskStreams::new("test_task", &output_dir).await.unwrap();

    // Write to both stdout and stderr
    let stdout_data = "stdout line\n";
    let stderr_data = "stderr line\n";

    let mut stdout_cursor = std::io::Cursor::new(stdout_data);
    let mut stderr_cursor = std::io::Cursor::new(stderr_data);

    // Process both streams
    streams
        .process_output(
            "test_task".to_string(),
            OutputType::Stdout,
            &mut stdout_cursor,
            false,
            false,
            TaskClocks::default().start("test_task"),
        )
        .await
        .unwrap();

    streams
        .process_output(
            "test_task".to_string(),
            OutputType::Stderr,
            &mut stderr_cursor,
            false,
            false,
            TaskClocks::default().start("test_task"),
        )
        .await
        .unwrap();

    let stdout_contents = streams.read_output(OutputType::Stdout).await.unwrap();
    let stderr_contents = streams.read_output(OutputType::Stderr).await.unwrap();

    assert_eq!(stdout_contents[0], "stdout line");
    assert_eq!(stderr_contents[0], "stderr line");
}

/// `--no-prefix` (docs/design/2026-08-28-boundary-fixes-and-dynamic-foreach.md
/// Phase 8): terminal output must drop the `[task]` prefix entirely, not
/// just the color, leaving exactly the task's own bytes.
#[test]
fn test_no_prefix_omits_task_prefix() {
    let out = format_terminal_output("loud-task", b"hello\n", true, true);
    assert_eq!(out, "hello\n");
}

/// Same call with `no_prefix: false` (the default) still carries the
/// task name, so a future regression that always suppresses the prefix
/// would fail this test.
#[test]
fn test_prefix_present_by_default() {
    let out = format_terminal_output("loud-task", b"hello\n", false, true);
    assert!(
        out.contains("loud-task"),
        "expected task name in prefixed output: {out:?}"
    );
    assert!(out.contains("hello\n"), "expected data to still be present: {out:?}");
}

/// **Phase 2 success criterion** (design doc
/// `docs/design/2026-09-15-idle-task-heartbeat.md`): a buffered foreach
/// subtask's bytes never reach the terminal (`suppress_terminal`), and its
/// idle clock must advance anyway. The clock answers "did this task produce a
/// line", not "did bytes reach the terminal", which is why the stamp sits
/// above the `suppress_terminal` branch: keyed on terminal writes, a
/// chattering buffered subtask would look idle and be heartbeated
/// continuously. A sibling's output must leave it alone - a sibling finishing
/// says nothing about whether this task is stuck.
#[tokio::test]
async fn a_suppressed_subtasks_line_advances_its_own_clock_and_no_siblings() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output_dir = PathBuf::from(temp_dir.path());

    let clocks = TaskClocks::default();
    let subtask = clocks.start("say:alpha");
    let sibling = clocks.start("build");
    let (subtask_before, sibling_before) = (subtask.last_line_ms(), sibling.last_line_ms());

    // A lower bound on elapsed time, which is the only kind a sleep
    // guarantees: it may overshoot, never undershoot. So the stamp taken
    // below is strictly later than the one `start` took, whatever the host's
    // clock resolution.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let streams = TaskStreams::new("say:alpha", &output_dir).await.unwrap();
    let mut cursor = std::io::Cursor::new("chatter\n");
    streams
        .process_output(
            "say:alpha".to_string(),
            OutputType::Stdout,
            &mut cursor,
            true,
            false,
            subtask.clone(),
        )
        .await
        .unwrap();

    assert!(
        subtask.last_line_ms() >= subtask_before + 20,
        "a suppressed subtask's line must still reset its clock: {} vs {subtask_before}",
        subtask.last_line_ms()
    );
    assert_eq!(
        sibling.last_line_ms(),
        sibling_before,
        "another task's output must not reset this task's clock"
    );
}

/// The stderr half of the leak this function used to carry: a chunk headed for
/// a stream that takes no colour gets the plain `[task]` prefix, escape-free.
/// Pinned here rather than only end-to-end because the function is pure for
/// exactly this reason - no terminal needed to test it.
#[test]
fn test_prefix_is_plain_for_a_stream_that_takes_no_colour() {
    let out = format_terminal_output("loud-task", b"hello\n", false, false);
    assert_eq!(out, "[loud-task] hello\n");
}

/// Its twin: with `takes_color` set the prefix is whatever `colored` decides,
/// which is the stdout behaviour the fix must leave alone. Compared against
/// `colorize_task_prefix` rather than against literal escapes, so it holds
/// under `cargo test` (no terminal) and under a pty alike.
#[test]
fn test_prefix_defers_to_colored_for_a_stream_that_takes_colour() {
    let out = format_terminal_output("loud-task", b"hello\n", false, true);
    assert_eq!(
        out,
        format!("{} hello\n", crate::executor::colors::colorize_task_prefix("loud-task"))
    );
}
