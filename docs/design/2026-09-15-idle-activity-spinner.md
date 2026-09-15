# Idle activity spinner

## The ask, and the constraint that shaped it

> can you add a spinner indicator when an action is in flight? the best example
> I have is the philo slim-dump. tty shows the download progress but otto does
> not, so it sits on that step for a few minutes. ideally otto knows when an
> action is in flight and nothing is being printed, and could do a little ascii
> spinner to let the user know that it isn't just frozen
>
> — and later: *if a process is running and there hasn't been a line logged in
> the last 5 (3?) seconds, start outputting a spinner to show activity*

The immediate objection, and the right one:

> if youre piping its output to a log, the spinner will spam the log with each
> `|` `/` `--` `\` ?
>
> — answered: *Ideally it wouldn't. If there's no way to show the spinner
> without defacing logs then it's not worth the visual indication*

So the feature is only worth having if captured output is provably untouched.
That is the bar this document is written against, and
`tests/progress_spinner_test.rs` is where it is enforced rather than asserted.

## The rule

**Frames go to stderr, and only when `stderr().is_terminal()`.**

This is what `git clone`, `cargo`, `docker build`, `pip` and `ninja` all do, and
it is the reason `git clone … 2>&1 | cat` shows no progress at all.

Three otto-specific reasons it has to be stderr rather than stdout or
`/dev/tty`:

- **stdout is otto's data channel.** `TeeWriter` sends a child's stdout to
  otto's stdout (`output.rs`), as do status lines and replayed blocks
  (`replay.rs`). Downstream consumers parse it — otto-dev's `ttv status --json`
  emits JSON Lines. A frame there corrupts it.
- **`/dev/tty` would contradict otto's own design.** Non-`tty` children are put
  in their own session precisely so they *cannot* reach the controlling
  terminal (`task_execution.rs`, `setsid`). otto reaching around a redirect to
  do it itself would also defeat `otto build 2>/dev/null`.
- **`is_terminal()` on stderr covers every capture route at once**: `2>&1 |`,
  `2>file`, `$( … 2>&1 )`, and a nested otto — whose stderr is a pipe because
  children are spawned with `Stdio::piped()`. Only the outermost interactive
  otto can ever draw.

One asymmetry, stated rather than hidden: under `otto build | tee log`, stdout
is a pipe but stderr is still a terminal, so frames render. That is correct and
conventional — the human gets progress, the pipe gets clean bytes.

## The second rule: silence first

A spinner that runs from the first moment spends its life being erased and
redrawn around output that is already telling the user what is happening. This
one waits **three seconds of silence** before drawing anything.

That is what was asked for, and it also makes the interleaving question nearly
moot: by construction there is usually nothing to interleave with. A chatty run
never spins at all.

## Where the erase discipline lives

Not at the call sites. otto already has a process-wide `TERMINAL_LOCK` whose
doc comment describes this exact class of problem — keeping a replayed foreach
block contiguous against concurrent writers — and every site that writes to
otto's own stdout or stderr already takes it.

So the lock owns the spinner's visibility: `terminal_lock()` erases any frame on
acquire and notes the write on release. Three physical acquisition sites cover
all seven logical writers, and none of them changed shape. A replayed block
holds the lock for its whole length, so the frame stays gone for the block
rather than flickering between its lines.

`terminal_lock_raw()` is the same lock without the hooks, for the ticker — which
must not reset the idle clock it is waiting on.

## When it is off

| Condition | Reason |
|---|---|
| stderr is not a terminal | the whole rule |
| `--no-progress` / `OTTO_NO_PROGRESS` | explicit opt-out, for a tty being recorded (`script`, `asciinema`, tmux capture) |
| `CI` non-empty | a build log is not a terminal in the sense that matters |
| `TERM` unset or `dumb` | `is_terminal()` does not check this, and a dumb terminal cannot erase a line |
| `NO_COLOR` | otto already honours it for prefixes; animating while suppressing colour is incoherent |
| `--tui` | the TUI owns the alternate screen and suppresses every other terminal write |

**Not** coupled to `--no-prefix`. That flag is about the shape of stdout for a
downstream consumer; this one is about whether stderr is a terminal. otto-dev's
`ttv` runs `otto --no-prefix` *interactively*, so coupling them would remove the
spinner from the case it exists for.

## Shape

One summary line, not one per task. otto runs tasks concurrently (`-j` defaults
to `available_parallelism()`; otto-dev uses 8, and `logs` fans out across
twelve services), and a multi-line region must be fully erased and redrawn on
every output line — while output is flushed per line by design. One line is
O(1) escape traffic per write.

```
⠹ 3 running: philo, catalog-api, auth-svc · 0:42
```

Truncated to `width - 1`: a frame that wraps occupies two rows while `\r` plus
clear-to-end-of-line erases one, and the leftover half-line is exactly the
defaced output this is not allowed to produce.

No frame ever carries a newline. A line that is never terminated cannot enter
scrollback.

## Known limits

- **A recorded session records frames.** Under `script`, `asciinema` or a tmux
  capture of a live pane, stderr genuinely *is* a pty. That cannot be detected
  and is not faked around; it is what `--no-progress`, `CI`, `NO_COLOR` and
  `TERM=dumb` are for.
- **A `tty: true` task inherits stdio and bypasses the terminal lock.** Such a
  task already takes the scheduler's entire permit count, so no other task runs
  alongside it — and with the idle rule a spinner only appears after three
  silent seconds. An interactive prompt that sits longer than that could still
  be drawn over; suspending explicitly around the exclusive-permit window is
  the obvious follow-up.
- **Windows is untested.** otto's terminal discipline is unix-shaped and the pty
  test helper is `script`-based.
