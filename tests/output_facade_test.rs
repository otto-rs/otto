//! The output facade is the ONLY terminal writer in the executor.
//!
//! Phase 2 of `docs/design/2026-09-16-live-progress-renderer.md` routed every
//! `print!`/`eprint!` in `src/executor/` through `progress::facade`. That is an
//! invariant, not a one-time cleanup: Phase 5 puts a redrawn region on the same
//! terminal, and one ad-hoc writer re-added later is how the region gets
//! scribbled over. The doc states it as an acceptance criterion; this pins it
//! so the criterion cannot quietly stop being true between phases.
//!
//! Deliberately the SAME spellings as the doc's canonical grep, including its
//! `!*_tests*.rs` exclusion (this repo splits tests as `scheduler_tests_b.rs`,
//! which a narrower `*_tests.rs` glob misses) and its omission of `write_all`
//! (which also names the authorized non-terminal writer that fills the per-task
//! log files).

use std::path::{Path, PathBuf};

/// The one file allowed to hold the macros, because it is the facade.
const FACADE: &str = "src/executor/progress/facade.rs";

const WRITER_MACROS: [&str; 4] = ["eprintln!", "println!", "eprint!", "print!"];

fn executor_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/executor");
    let mut found = Vec::new();
    collect(&root, &mut found);
    assert!(
        found.len() > 10,
        "walked {} files under src/executor; the walk is broken, not the tree",
        found.len()
    );
    found
}

fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("src/executor is readable") {
        let path = entry.expect("readable dir entry").path();
        if path.is_dir() {
            collect(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            // `*_tests*.rs`, matching the doc's glob: a test may print.
            if !name.contains("_tests") {
                found.push(path);
            }
        }
    }
}

fn relative(path: &Path) -> String {
    path.strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap_or(path)
        .display()
        .to_string()
}

#[test]
fn no_executor_source_outside_the_facade_writes_to_the_terminal_directly() {
    let mut offenders = Vec::new();

    for path in executor_sources() {
        let rel = relative(&path);
        if rel == FACADE {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("source file is UTF-8");
        for (n, line) in source.lines().enumerate() {
            if WRITER_MACROS.iter().any(|m| line.contains(m)) {
                offenders.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these bypass the output facade; route them through `progress::facade` \
         (a doc comment naming a macro counts, because the acceptance grep counts it):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_process_wide_terminal_lock_is_gone() {
    let mut offenders = Vec::new();

    for path in executor_sources() {
        let source = std::fs::read_to_string(&path).expect("source file is UTF-8");
        for (n, line) in source.lines().enumerate() {
            if line.contains("TERMINAL_LOCK") || line.contains("terminal_lock") {
                offenders.push(format!("{}:{}: {}", relative(&path), n + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "the facade replaced the process-wide terminal lock; a surviving reference means \
         either a second ordering authority or a doc comment pointing at a deleted symbol:\n{}",
        offenders.join("\n")
    );
}
