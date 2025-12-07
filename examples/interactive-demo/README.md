# Interactive TTY Demo

This example demonstrates Otto's interactive task support with PTY (pseudo-terminal).

## What is Interactive Mode?

When a task is marked with `interactive: true`, Otto:
- Allocates a PTY (pseudo-terminal) for full terminal control
- Passes stdin directly to the task (you can type, use arrow keys, Ctrl+C, etc.)
- Preserves ANSI colors and terminal formatting
- Logs all I/O to `interactive.log` for session replay
- Serializes interactive tasks (only one runs at a time)
- Automatically disables TUI mode if enabled

## Examples

### Simple Input Test
```bash
otto read-input
```
Prompts for your name and echoes it back. Simple example of reading user input.

### Interactive Shell
```bash
otto shell
```
Launches a full bash shell. You can:
- Run commands
- Use tab completion
- Use command history (arrow keys)
- Use Ctrl+C to interrupt commands
- Type `exit` or Ctrl+D to finish

### Text Editor
```bash
otto vim-edit
```
Opens vim to edit a file. Full terminal control:
- All vim keybindings work
- Colors displayed correctly
- Modal editing works as expected
- `:wq` to save and quit

### Python REPL
```bash
otto python-interactive
```
Interactive Python shell with full readline support.

### System Monitor
```bash
otto top-monitor
# or
otto htop-monitor
```
Interactive process monitor with:
- Real-time updates
- Keyboard navigation
- Colors and formatting

### Colored Menu
```bash
otto colored-menu
```
Demonstrates ANSI color codes working correctly in interactive mode.

### Regular Task (Comparison)
```bash
otto echo-test
```
A normal non-interactive task for comparison. Output is captured but no interaction.

## Testing Multiple Tasks

Interactive tasks are serialized:
```bash
# This will run shell, then read-input sequentially
otto shell read-input
```

Mix with non-interactive tasks:
```bash
# echo-test can run in parallel, but interactive tasks serialize
otto echo-test shell read-input
```

## Session Logs

All interactive I/O is logged to:
```
~/.otto/workspaces/<workspace_hash>/runs/<run_timestamp>/tasks/<task_name>/interactive.log
```

View your session history:
```bash
otto history shell
```

## TUI Mode

If you try to use `--tui` with interactive tasks, Otto will automatically disable TUI:
```bash
otto --tui shell
# Warning: Interactive tasks require full terminal access, disabling TUI
```

## Requirements

- Unix-like system (Linux, macOS)
- Tasks marked with `interactive: true`
- Terminal with TTY support

## What This Proves

✅ Full terminal control (readline, arrow keys, Ctrl+C)
✅ ANSI colors and formatting preserved
✅ Complex interactive programs work (vim, top, shells)
✅ Session logging captures everything
✅ Terminal always restored after task
✅ Graceful Ctrl+C handling
✅ Serialized execution prevents conflicts


