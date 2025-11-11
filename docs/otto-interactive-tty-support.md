# Otto Interactive TTY Support

## Problem Statement

Otto currently cannot execute interactive commands that require bidirectional terminal I/O (stdin/stdout/stderr passthrough with TTY support). Commands like `docker compose exec -it app /bin/bash`, `psql`, or any REPL fail because:

1. Otto intercepts and redirects stdio for logging and formatting
2. Child processes don't receive a proper controlling terminal (TTY)
3. The `interactive: true` flag **does not exist** in the current TaskSpec schema and needs to be added

This prevents otto from being a complete replacement for Make, which handles interactive commands transparently.

## Goals

1. **Full Interactive Support**: Commands with `interactive: true` should work identically to direct shell execution
2. **Complete Logging**: All I/O (including interactive sessions) should be captured to `~/.otto/` history
3. **Zero Compromises**: No loss of existing otto features (prefixes, TUI, DAG, parallelism)
4. **Cross-Platform**: Work on Linux, macOS, and Windows
5. **Transparent to Users**: No special syntax beyond `interactive: true` flag

## Non-Goals

1. Multiple simultaneous interactive tasks (serialize them automatically)
2. Interactive tasks in background mode (force foreground)
3. TUI dashboard during interactive sessions (suspend TUI, resume after)
4. Prefix formatting for interactive output (raw passthrough only)

## Architecture Overview

### Current Flow (Non-Interactive)

```
User → Otto Process → Spawned Task (piped stdio)
                  ↓
            Logging + Prefixes + TUI
                  ↓
            User's Terminal
```

### Proposed Flow (Interactive with PTY)

```
User Terminal (real TTY)
       ↕ (bidirectional, transparent)
Otto Process (PTY Proxy)
       ↕ (PTY master/slave pair)
Task Process (sees real TTY)

       ↓ (tee'd copy)
    ~/.otto/logs/ (full session recording)
```

## Detailed Design

### 1. PTY (Pseudo-TTY) Implementation

**What is a PTY?**
- A PTY is a kernel-level terminal emulator consisting of:
  - **Master side**: Controlled by the "terminal emulator" (otto)
  - **Slave side**: Appears as `/dev/pts/N` to the child process
- The child process believes it's connected to a real terminal

**How PTYs Enable Full Functionality:**
- Child process `isatty()` checks return true
- Terminal control sequences (colors, cursor movement) work
- Line editing (readline, tab completion) works
- Signal handling (Ctrl+C, Ctrl+Z) works
- Window size (SIGWINCH) propagates correctly

### 2. Implementation Components

#### Component A: PTY Allocator

```rust
struct InteractivePty {
    master: PtyMaster,
    slave: PtySlave,
    original_termios: Termios,
}

impl InteractivePty {
    fn new() -> Result<Self> {
        // Use portable-pty crate for cross-platform support
        let pty_pair = portable_pty::native_pty_system()
            .openpty(PtySize::default())?;

        // Save original terminal settings
        let original_termios = termios::tcgetattr(stdin())?;

        // Set terminal to raw mode (disable line buffering, echo, etc.)
        let mut raw = original_termios.clone();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(stdin(), termios::SetArg::TCSANOW, &raw)?;

        Ok(Self {
            master: pty_pair.master,
            slave: pty_pair.slave,
            original_termios,
        })
    }

    fn restore_terminal(&self) -> Result<()> {
        // Restore original terminal settings on exit
        termios::tcsetattr(stdin(), termios::SetArg::TCSANOW, &self.original_termios)
    }
}

impl Drop for InteractivePty {
    fn drop(&mut self) {
        let _ = self.restore_terminal();
    }
}
```

#### Component B: Bidirectional I/O Proxy

```rust
struct IoProxy {
    pty_master: PtyMaster,
    log_buffer: Arc<Mutex<Vec<u8>>>,
}

impl IoProxy {
    async fn run(&mut self) -> Result<()> {
        let stdin_handle = tokio::spawn(self.proxy_stdin_to_pty());
        let stdout_handle = tokio::spawn(self.proxy_pty_to_stdout());

        // Wait for both directions (either can end first)
        tokio::select! {
            _ = stdin_handle => {},
            _ = stdout_handle => {},
        }

        Ok(())
    }

    async fn proxy_stdin_to_pty(&mut self) -> Result<()> {
        let mut stdin = tokio::io::stdin();
        let mut buf = [0u8; 4096];

        loop {
            let n = stdin.read(&mut buf).await?;
            if n == 0 { break; } // EOF

            self.pty_master.write_all(&buf[..n]).await?;
        }
        Ok(())
    }

    async fn proxy_pty_to_stdout(&mut self) -> Result<()> {
        let mut stdout = tokio::io::stdout();
        let mut buf = [0u8; 4096];

        loop {
            let n = self.pty_master.read(&mut buf).await?;
            if n == 0 { break; } // Child exited

            // Write to stdout (user sees output in real-time)
            stdout.write_all(&buf[..n]).await?;
            stdout.flush().await?;

            // Tee to log buffer for history
            self.log_buffer.lock().unwrap().extend_from_slice(&buf[..n]);
        }
        Ok(())
    }
}
```

#### Component C: Window Size Handling

```rust
use tokio::signal::unix::{signal, SignalKind};
use nix::libc;
use nix::ioctl_read_bad;
use nix::ioctl_write_ptr_bad;

// Define ioctl wrappers for window size
ioctl_read_bad!(read_winsize, libc::TIOCGWINSZ, libc::winsize);
ioctl_write_ptr_bad!(write_winsize, libc::TIOCSWINSZ, libc::winsize);

async fn setup_winch_handler(pty_fd: std::os::unix::io::RawFd) -> Result<()> {
    // Get current terminal size
    let mut winsize = unsafe { std::mem::zeroed::<libc::winsize>() };
    unsafe { read_winsize(libc::STDIN_FILENO, &mut winsize)? };

    // Set PTY slave to same size
    unsafe { write_winsize(pty_fd, &winsize)? };

    // Handle SIGWINCH (terminal resize) using tokio signals
    let mut sigwinch = signal(SignalKind::window_change())?;

    tokio::spawn(async move {
        while sigwinch.recv().await.is_some() {
            // Get new window size
            let mut new_winsize = unsafe { std::mem::zeroed::<libc::winsize>() };
            if unsafe { read_winsize(libc::STDIN_FILENO, &mut new_winsize).is_ok() } {
                // Update PTY window size
                let _ = unsafe { write_winsize(pty_fd, &new_winsize) };
            }
        }
    });

    Ok(())
}
```

#### Component D: Task Execution

```rust
async fn execute_interactive_task(
    task: &Task,
    script: &str,
) -> Result<TaskResult> {
    // Create PTY
    let pty = InteractivePty::new()?;

    // Spawn child process with PTY slave
    let mut child = Command::new("bash")
        .args(["-c", script])
        .stdin(Stdio::from(pty.slave.as_raw_fd()))
        .stdout(Stdio::from(pty.slave.as_raw_fd()))
        .stderr(Stdio::from(pty.slave.as_raw_fd()))
        .spawn()?;

    // Close slave in parent (only child needs it)
    drop(pty.slave);

    // Setup window size handling
    setup_winch_handler(&pty.master)?;

    // Start I/O proxy
    let log_buffer = Arc::new(Mutex::new(Vec::new()));
    let mut proxy = IoProxy {
        pty_master: pty.master,
        log_buffer: log_buffer.clone(),
    };

    // Run proxy and wait for child
    let proxy_handle = tokio::spawn(async move {
        proxy.run().await
    });

    let exit_status = child.wait().await?;
    proxy_handle.await??;

    // Restore terminal
    pty.restore_terminal()?;

    // Save log to history
    let log_data = log_buffer.lock().unwrap();
    save_task_log(&task.name, &log_data)?;

    Ok(TaskResult {
        exit_code: exit_status.code(),
        output: None, // No captured output for interactive tasks
        log_file: Some(get_log_path(&task.name)),
    })
}
```

### 3. Integration with Existing Otto Features

#### DAG and Dependencies

- **Before Dependencies**: Execute normally (e.g., `before: [up]`)
- **Interactive Task**: Suspend TUI, run in foreground
- **After Dependencies**: Resume normal execution

```yaml
shell:
  help: "Open shell"
  before: [up]        # These run normally with TUI
  interactive: true   # This suspends TUI, runs in foreground
  bash: docker compose exec -it app /bin/bash
```

#### Parallelism

- Interactive tasks **cannot run in parallel** (only one can have terminal control)
- Implementation: Use separate semaphore with permit count of 1 for interactive tasks
- Non-interactive tasks can run in parallel as usual

**Implementation in TaskScheduler:**

```rust
pub struct TaskScheduler {
    semaphore: Arc<Semaphore>,              // Normal tasks (max_parallel permits)
    interactive_semaphore: Arc<Semaphore>,  // Interactive tasks (1 permit only)
    // ... other fields
}

impl TaskScheduler {
    pub async fn new(..., max_parallel: usize, ...) -> Result<Self> {
        Ok(Self {
            semaphore: Arc::new(Semaphore::new(max_parallel)),
            interactive_semaphore: Arc::new(Semaphore::new(1)),  // Force serial execution
            // ... other fields
        })
    }
}

async fn execute_task(&self, task: Task, ...) -> Result<JoinHandle<Result<()>>> {
    // Choose semaphore based on task type
    let semaphore = if task.interactive {
        self.interactive_semaphore.clone()  // Serialize interactive tasks
    } else {
        self.semaphore.clone()  // Allow parallel execution
    };

    Ok(tokio::spawn(async move {
        let _permit = semaphore.acquire().await?;
        // ... rest of task execution
    }))
}
```

#### TUI Dashboard

**Strategy: Disable TUI for Interactive Tasks**

When any task in the execution graph has `interactive: true`, TUI mode is automatically disabled and execution falls back to standard terminal output. This is the simplest approach for MVP.

**Implementation in main.rs:**

```rust
async fn execute_tasks(
    tasks: Vec<Task>,
    hash: String,
    ottofile_path: Option<PathBuf>,
    jobs: usize,
    tui_mode: bool,
) -> Result<()> {
    if tui_mode {
        if !atty::is(atty::Stream::Stdout) {
            eprintln!("Warning: --tui requires a TTY, falling back to standard output");
            return execute_with_terminal_output(tasks, hash, ottofile_path, jobs).await;
        }

        // Check for interactive tasks
        if tasks.iter().any(|t| t.interactive) {
            eprintln!("Warning: Interactive tasks require full terminal access, disabling TUI");
            return execute_with_terminal_output(tasks, hash, ottofile_path, jobs).await;
        }

        execute_with_tui(tasks, hash, ottofile_path, jobs).await
    } else {
        execute_with_terminal_output(tasks, hash, ottofile_path, jobs).await
    }
}
```

**Rationale:**
- Simpler implementation (no suspend/resume complexity)
- Cleaner terminal state management
- Interactive tasks need full terminal control anyway
- Can upgrade to TUI suspend/resume in Phase 4+ if needed

#### Logging

- **Full I/O capture**: Everything written to `~/.otto/logs/{task_name}.log`
- **Raw format**: No `[task]` prefixes (preserve raw terminal sequences)
- **History entry**: Normal metadata (start time, end time, exit code)
- **Replay support**: Logs can be replayed with `cat` or `less -R` (preserves colors)

### 4. Configuration

#### YAML Schema

```yaml
tasks:
  shell:
    help: "Open interactive shell"
    before: [up]              # Normal dependencies
    interactive: true         # Enable PTY mode
    bash: |
      docker compose exec app /bin/bash

  psql:
    help: "Connect to PostgreSQL"
    before: [up]
    interactive: true
    bash: |
      docker compose exec db psql -U user -d database
```

#### Validation

- If `interactive: true`:
  - ✅ Allow: Single bash script
  - ❌ Disallow: Multiple commands in parallel
  - ❌ Disallow: `background: true` (interactive tasks must be foreground)
  - ⚠️  Warn: If task has `after` dependents, they'll wait

### 5. Error Handling

| Scenario | Behavior |
|----------|----------|
| PTY allocation fails | Fall back to piped stdio with warning |
| Terminal not available (CI/CD) | Error: "Interactive tasks require a TTY" |
| Child process exits abnormally | Restore terminal, log exit code |
| User sends Ctrl+C | Forward signal to child, wait for clean exit |
| Terminal resize during execution | Forward SIGWINCH to PTY |
| Multiple interactive tasks requested | Serialize automatically |

### 6. Platform Considerations

#### Linux
- Use `openpty()` from `libc`
- Full support for all features

#### macOS
- Use `openpty()` from `libc`
- Same as Linux

#### Windows
- Use Windows ConPTY API (available since Windows 10 1809)
- `portable-pty` crate handles platform differences
- May have slight behavioral differences (document in wiki)

### 7. Schema Changes Required

**CRITICAL:** These changes must be implemented before Phase 3 can begin.

#### src/cfg/task.rs

Add `interactive` field to TaskSpec:

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskSpec {
    pub name: String,
    pub help: Option<String>,
    pub after: Vec<String>,
    pub before: Vec<String>,
    pub input: Vec<String>,
    pub output: Vec<String>,
    pub envs: HashMap<String, String>,
    pub params: ParamSpecs,
    pub action: String,
    pub interactive: Option<bool>,  // ADD THIS FIELD
}

#[derive(Debug, Deserialize)]
struct TaskSpecHelper {
    #[serde(default)]
    help: Option<String>,
    // ... other existing fields ...

    #[serde(default)]
    interactive: Option<bool>,  // ADD THIS FIELD
}
```

Update deserialization to pass through interactive flag:

```rust
impl<'de> Deserialize<'de> for TaskSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let helper = TaskSpecHelper::deserialize(deserializer)?;

        // ... existing action determination logic ...

        Ok(TaskSpec {
            name: String::new(),
            help: helper.help,
            after: helper.after,
            before: helper.before,
            input: helper.input,
            output: helper.output,
            envs: helper.envs,
            params: helper.params,
            action,
            interactive: helper.interactive,  // ADD THIS
        })
    }
}
```

#### src/executor/task.rs

Add `interactive` field to Task:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub name: String,
    pub task_deps: Vec<String>,
    pub file_deps: Vec<String>,
    pub output_deps: Vec<String>,
    pub envs: HashMap<String, String>,
    pub values: HashMap<String, Value>,
    pub action: String,
    pub hash: String,
    pub interactive: bool,  // ADD THIS FIELD
}

impl Task {
    #[must_use]
    pub fn from_task_with_cwd_and_global_envs(
        task_spec: &TaskSpec,
        cwd: &std::path::Path,
        global_envs: &HashMap<String, String>,
    ) -> Self {
        // ... existing code ...

        Self::new(
            name,
            task_deps,
            file_deps,
            output_deps,
            evaluated_envs,
            values,
            action,
            task_spec.interactive.unwrap_or(false),  // ADD THIS - default to non-interactive
        )
    }
}
```

Update Task::new() signature:

```rust
pub fn new(
    name: String,
    task_deps: Vec<String>,
    file_deps: Vec<String>,
    output_deps: Vec<String>,
    envs: HashMap<String, String>,
    values: HashMap<String, Value>,
    action: String,
    interactive: bool,  // ADD THIS PARAMETER
) -> Self {
    let hash = calculate_hash(&action);
    Self {
        name,
        task_deps,
        file_deps,
        output_deps,
        envs,
        values,
        action,
        hash,
        interactive,  // ADD THIS
    }
}
```

#### Validation Logic

Add validation in TaskSpec:

```rust
impl TaskSpec {
    pub fn validate(&self) -> Result<()> {
        if self.interactive == Some(true) {
            if self.action.is_empty() {
                return Err(eyre!("Interactive task '{}' must have an action", self.name));
            }

            // Warn about dependencies
            if !self.after.is_empty() {
                eprintln!(
                    "Warning: Interactive task '{}' has 'after' dependencies. \
                     Dependent tasks will wait for interactive completion.",
                    self.name
                );
            }
        }
        Ok(())
    }
}
```

## Implementation Plan

### Phase 1: Core PTY Infrastructure
1. Add `portable-pty` and `termios` dependencies
2. Implement `InteractivePty` allocator
3. Implement raw terminal mode handling
4. Add terminal restoration on panic/exit
5. Unit tests for PTY allocation

### Phase 2: I/O Proxy
1. Implement bidirectional async I/O proxy
2. Add buffer/logging for all I/O
3. Implement window size change handling (SIGWINCH)
4. Handle edge cases (EOF, child exit, errors)
5. Integration tests with simple interactive commands

### Phase 3: Task Execution Integration
**PREREQUISITE:** Must complete Schema Changes (Section 7) first

1. Add `interactive: bool` field to TaskSpec and Task structs (see Section 7)
2. Route to PTY-based executor when `task.interactive == true`
3. Implement interactive_semaphore for serialization (see Parallelism section)
4. Integrate with existing DAG execution
5. Add validation for invalid configurations

### Phase 4: TUI Integration
1. Implement TUI disable check (see TUI Dashboard section)
2. Add detection for interactive tasks in execution graph
3. Display warning message when TUI is disabled
4. Test TUI + interactive task combinations
5. Optional future enhancement: TUI suspend/resume instead of disable

### Phase 5: Logging & History
1. Save interactive session logs to `~/.otto/logs/`
2. Preserve raw terminal sequences in logs
3. Add metadata to history (interactive flag, session duration)
4. Implement log replay/viewing

### Phase 6: Polish & Edge Cases
1. Handle Ctrl+C gracefully
2. Add timeout support for interactive tasks
3. Improve error messages
4. Add platform-specific documentation
5. Performance testing (large output)

## Testing Strategy

### Unit Tests
- PTY allocation and cleanup
- Terminal mode save/restore
- Window size propagation
- Buffer/logging without actual TTY

### Integration Tests
```rust
#[test]
fn test_interactive_shell_echo() {
    // Spawn interactive task with script that echoes input
    let mut task = create_interactive_task("echo 'hello' | cat");
    let result = execute(task).await.unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result.log_file.exists());

    let log_content = fs::read_to_string(result.log_file).unwrap();
    assert!(log_content.contains("hello"));
}

#[test]
fn test_interactive_with_dependencies() {
    let tasks = vec![
        create_task("setup", "echo 'setting up'"),
        create_interactive_task_with_deps("shell", "bash", vec!["setup"]),
    ];

    let result = execute_dag(tasks).await.unwrap();

    // Ensure setup ran before shell
    assert!(result.execution_order == vec!["setup", "shell"]);
}

#[test]
fn test_terminal_restoration_on_error() {
    let original_termios = get_terminal_settings();

    let task = create_interactive_task("shell", "exit 1");
    let _ = execute(task).await; // Expect failure

    let restored_termios = get_terminal_settings();
    assert_eq!(original_termios, restored_termios);
}
```

### Manual Tests
- Run `otto shell` and verify tab completion works
- Run `otto psql` and verify colors/formatting work
- Resize terminal during `otto shell` and verify it resizes correctly
- Send Ctrl+C during interactive task and verify clean exit
- Run on Linux, macOS, Windows and verify consistent behavior

### CI/CD Tests
- Verify error message when no TTY available
- Test in Docker container without TTY
- Test in GitHub Actions (should fail gracefully)

## Dependencies

### Rust Crates

```toml
[dependencies]
# Cross-platform PTY support
portable-pty = "0.8"

# Async I/O (already in otto)
tokio = { version = "1.48.0", features = ["full"] }

[target.'cfg(unix)'.dependencies]
# Unix-specific terminal control and file descriptor operations
nix = { version = "0.27", features = ["term", "ioctl", "process", "signal"] }
```

**Note on Signal Handling:**
- Use `tokio::signal::unix::signal(SignalKind::window_change())` for SIGWINCH
- Otto already has tokio with full features, no need for `signal-hook`
- Example:
```rust
use tokio::signal::unix::{signal, SignalKind};

let mut sigwinch = signal(SignalKind::window_change())?;
tokio::spawn(async move {
    while sigwinch.recv().await.is_some() {
        // Handle window resize
    }
});
```

**Note on Platform Support:**
- Unix (Linux/macOS): Full support via `nix` crate
- Windows: Future phase using `portable-pty` ConPTY backend

## Performance Considerations

### Buffering
- Use 4KB buffers for I/O proxy (standard page size)
- Don't buffer entire sessions in memory (stream to disk)
- Flush stdout after each write (for responsiveness)

### Memory
- Interactive sessions can generate lots of output (e.g., log tails)
- Stream directly to log files, not memory buffers
- Use `mmap` for large log files if needed

### CPU
- I/O copying is negligible overhead
- PTY overhead is minimal (kernel-level)
- Async I/O prevents blocking

## Security Considerations

### Terminal Escape Sequences
- Interactive commands can output malicious escape sequences
- **Don't sanitize**: Breaks legitimate use cases (colors, cursor movement)
- **Document risk**: Users should trust interactive commands
- **Future**: Add opt-in escape sequence filtering

### Log File Access
- Logs may contain sensitive data (passwords, tokens)
- Already handled by `~/.otto/` permissions (user-only)
- Document: Don't share interactive session logs

### PTY Security
- PTYs don't provide process isolation
- Child can detect it's in a PTY (not a concern for legitimate use)
- Can't prevent child from detecting it's being logged

## Documentation Additions

### User-Facing Docs

#### Basic Usage
```yaml
# .otto.yml
tasks:
  shell:
    help: "Open shell in container"
    before: [up]
    interactive: true
    bash: docker compose exec app /bin/bash
```

Run with: `otto shell`

#### Troubleshooting
- **"Interactive tasks require a TTY"**: Running in CI/CD without terminal
- **Terminal corrupted after crash**: Run `reset` command
- **Tab completion doesn't work**: Check that `-it` flags are passed to docker

### Developer Docs
- Architecture diagram of PTY flow
- How to debug PTY issues
- Platform-specific gotchas
- Contributing guide for PTY-related changes

## Future Enhancements

### Interactive Task Recording (v2)
- Record interactive sessions like `asciinema`
- Replay sessions with timing information
- Share sessions as `.cast` files

### Multi-User Collaboration (v3)
- Multiple users attach to same interactive session
- Like `tmux` attach/detach
- Useful for pair programming

### Interactive Task Timeout (v2)
```yaml
shell:
  interactive: true
  timeout: 30m  # Auto-exit after 30 minutes
  bash: docker compose exec app /bin/bash
```

### Nested PTY Support (v3)
- Running `otto` inside an interactive otto task
- Requires PTY-in-PTY handling

## Alternatives Considered

### 1. No Interactive Support (Status Quo)
**Rejected**: Limits otto's usefulness, forces users to remember direct commands

### 2. Simple stdin/stdout Inherit
```rust
Command::new("bash")
    .stdin(Stdio::inherit())
    .stdout(Stdio::inherit())
    .spawn()
```
**Rejected**: No logging, no TTY support (isatty returns false), breaks tab completion

### 3. Wrapper Script Generation
Generate temporary shell script, tell user to run it manually
**Rejected**: Breaks workflow, defeats purpose of task runner

### 4. Use `script` Command
Call out to Unix `script` command for recording
**Rejected**: Platform-specific, less control, extra dependency

## Success Metrics

- ✅ `docker compose exec -it app /bin/bash` works via otto
- ✅ Tab completion works
- ✅ Colors and formatting preserved
- ✅ Window resize handled correctly
- ✅ Full session logged to `~/.otto/logs/`
- ✅ Terminal restored cleanly on exit/error
- ✅ Works on Linux, macOS, Windows
- ✅ Zero user complaints about interactive tasks

## References

### PTY Resources
- [The TTY Demystified](https://www.linusakesson.net/programming/tty/)
- [Linux PTY(7) Man Page](https://man7.org/linux/man-pages/man7/pty.7.html)
- [portable-pty Documentation](https://docs.rs/portable-pty/)

### Prior Art
- **tmux**: Session management with PTY
- **asciinema**: Terminal recording with PTY
- **script**: Unix command for logging sessions
- **expect**: Automating interactive commands

### Rust Examples
- [alacritty](https://github.com/alacritty/alacritty): Terminal emulator using PTY
- [zellij](https://github.com/zellij-org/zellij): Terminal multiplexer
- [rustyline](https://github.com/kkawakam/rustyline): Readline implementation

---

**Document Status**: Implementation Ready
**Author**: Otto Core Team
**Last Updated**: 2025-11-11
**Revision**: 2 - Critical fixes applied

## Changelog

### Revision 2 (2025-11-11)
**Critical fixes to make document implementation-ready:**

1. **Fixed**: Line 9 - Corrected false statement that `interactive: true` exists. Documented that field must be added to schema.

2. **Added**: Section 7 - Complete schema changes required for TaskSpec and Task structs with exact code changes needed.

3. **Fixed**: Parallelism section - Added concrete implementation using `interactive_semaphore` for serialization.

4. **Fixed**: TUI Dashboard section - Made clear decision to disable TUI (not suspend/resume) for MVP.

5. **Fixed**: Dependencies - Removed `signal-hook`, using `tokio::signal` instead (Otto already has this).

6. **Updated**: Component C - Window size handling now uses tokio signals correctly.

7. **Updated**: Phase 3 & 4 - Added prerequisites and clarified TUI disable strategy.

