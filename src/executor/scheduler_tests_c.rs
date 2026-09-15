#![cfg(test)]
#![cfg(unix)]

//! Phase 2 (design doc 2026-09-15, idle task heartbeat). The tick-time
//! candidate set, read the way the ticker Phase 3 adds will read it: liveness
//! from the live-child registry the task bodies write, timing from the clock
//! map, and nothing mirrored between them.
//!
//! **Every claim here is driven by a fifo, never waited out on a stopwatch.**
//! Each task blocks until this test releases it, so "two children are live" is
//! a fact about a barrier rather than about how long anything took, and a
//! regression that started a third would hold that state until the test gave
//! up instead of flashing past a sampler.

use super::*;
use serial_test::serial;
use std::collections::HashSet;
use std::io::Write;
use std::num::NonZeroUsize;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use tempfile::TempDir;

/// Point this test's otto home at a scratch directory. See `tests_a` for why
/// `OTTO_DB_PATH` is cleared rather than set.
fn setup_test_db(temp_dir: &std::path::Path) {
    let otto_home = temp_dir.join(".otto");
    // SAFETY: This is safe in tests because we control the execution environment
    // and tests are isolated. The env var is set before any StateManager is created.
    unsafe {
        std::env::remove_var("OTTO_DB_PATH");
        std::env::set_var("OTTO_HOME", &otto_home);
    }
}

/// How long any single barrier gets before the test gives up. A bound on
/// failure, not a measurement: every barrier is released by this test.
const STEP_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a whole run gets once the last barrier is released.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

/// How often a barrier, or the candidate set, is read.
const POLL: Duration = Duration::from_millis(5);

/// One task that announces itself and then blocks until released.
///
/// A fifo, not a sleep: the task is unblocked by this test writing to it,
/// so the run makes progress only when the test says so. Silent while it
/// blocks, which is also the shape the heartbeat exists for.
struct Blocker {
    marker: PathBuf,
    fifo: PathBuf,
}

impl Blocker {
    fn new(dir: &Path, name: &str) -> Self {
        let blocker = Self {
            marker: dir.join(format!("{name}.started")),
            fifo: dir.join(format!("{name}.fifo")),
        };
        let status = std::process::Command::new("mkfifo")
            .arg(&blocker.fifo)
            .status()
            .expect("mkfifo must be available on a unix host");
        assert!(status.success(), "mkfifo failed for {}", blocker.fifo.display());
        blocker
    }

    fn action(&self) -> String {
        format!(
            "touch {}\ncat {} >/dev/null",
            self.marker.display(),
            self.fifo.display()
        )
    }

    fn started(&self) -> bool {
        self.marker.exists()
    }

    /// Unblock whatever is reading the fifo, and say whether anything was.
    ///
    /// `O_NONBLOCK`, which is what makes this safe to call on a fifo nobody is
    /// reading: the open fails with `ENXIO` instead of blocking this test
    /// forever. That is also why it reports - the task writes its marker
    /// before it reaches its `cat`, so the first attempt can arrive ahead of
    /// the reader, and a caller that needs the task actually released must
    /// retry rather than believe one attempt.
    fn release(&self) -> bool {
        match std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&self.fifo)
        {
            Ok(mut fifo) => fifo.write_all(b"go\n").is_ok(),
            Err(_) => false,
        }
    }

    /// Release, retrying until the task has actually reached its `cat`.
    async fn released(&self) -> bool {
        wait_for(|| self.release()).await
    }
}

/// Best-effort release of every fifo, so a failing assertion cannot leave a
/// blocked `cat` behind. Unconditional and unchecked on purpose: it runs on
/// paths where some of these tasks were never started at all.
fn release_all(blockers: &[&Blocker]) {
    for blocker in blockers {
        blocker.release();
    }
}

/// Poll a file barrier until it holds, or give up and say so. Returning
/// rather than panicking is deliberate: the caller releases its fifos
/// before it fails, because a blocked `cat` outlives a panicking test.
async fn wait_for(mut barrier: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + STEP_TIMEOUT;
    while Instant::now() < deadline {
        if barrier() {
            return true;
        }
        tokio::time::sleep(POLL).await;
    }
    false
}

/// The tick-time candidate set, by name.
async fn candidate_names(scheduler: &TaskScheduler) -> Vec<String> {
    scheduler
        .tick_candidates()
        .await
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// Poll the candidate set until it is exactly `expected`.
async fn wait_for_candidates(scheduler: &TaskScheduler, expected: &[&str]) -> bool {
    let deadline = Instant::now() + STEP_TIMEOUT;
    while Instant::now() < deadline {
        if candidate_names(scheduler).await == expected {
            return true;
        }
        tokio::time::sleep(POLL).await;
    }
    false
}

/// Poll until the scheduler has recorded `task` as completed.
async fn wait_for_completion(scheduler: &TaskScheduler, task: &str) -> bool {
    let deadline = Instant::now() + STEP_TIMEOUT;
    while Instant::now() < deadline {
        if scheduler.get_task_status(task).await == TaskStatus::Completed {
            return true;
        }
        tokio::time::sleep(POLL).await;
    }
    false
}

async fn scheduler_for(tasks: Vec<Task>, work_dir: PathBuf, max_parallel: usize) -> Result<Arc<TaskScheduler>> {
    let workspace = Workspace::new(work_dir).await?;
    workspace.init().await?;
    Ok(Arc::new(
        TaskScheduler::new(tasks, Arc::new(workspace), ExecutionContext::new(), max_parallel, false).await?,
    ))
}

fn blocking_task(name: &str, blocker: &Blocker) -> Task {
    Task::new(
        name.to_string(),
        None,
        vec![],
        vec![],
        vec![],
        HashMap::new(),
        HashMap::new(),
        blocker.action(),
    )
}

fn run(scheduler: &Arc<TaskScheduler>) -> tokio::task::JoinHandle<Result<()>> {
    let scheduler = scheduler.clone();
    tokio::spawn(async move { scheduler.execute_all().await })
}

/// Sample the two tick-time reads for as long as it is running, keeping
/// the largest count each one ever showed.
struct Sampler {
    max_live: Arc<AtomicUsize>,
    max_candidates: Arc<AtomicUsize>,
    handle: tokio::task::JoinHandle<()>,
}

impl Sampler {
    fn watching(scheduler: Arc<TaskScheduler>) -> Self {
        let max_live = Arc::new(AtomicUsize::new(0));
        let max_candidates = Arc::new(AtomicUsize::new(0));
        let handle = {
            let (max_live, max_candidates) = (max_live.clone(), max_candidates.clone());
            tokio::spawn(async move {
                loop {
                    max_live.fetch_max(scheduler.live_child_names().await.len(), Ordering::Relaxed);
                    max_candidates.fetch_max(scheduler.tick_candidates().await.len(), Ordering::Relaxed);
                    tokio::time::sleep(POLL).await;
                }
            })
        };
        Self {
            max_live,
            max_candidates,
            handle,
        }
    }

    /// The two high-water marks, sampling stopped.
    fn stop(self) -> (usize, usize) {
        self.handle.abort();
        (
            self.max_live.load(Ordering::Relaxed),
            self.max_candidates.load(Ordering::Relaxed),
        )
    }
}

/// **Phase 2 success criterion.** A `foreach ... jobs: 2` group over 8
/// items reaches 2 live children at tick time and never exceeds 2.
///
/// Both halves matter. The upper bound is the choice of ledger: a
/// heartbeat read off `ActiveTasks` rather than the child registry would
/// see all 8, because `spawn` counts an item in flight while its body is
/// still queuing for its group's permit. The lower bound is the half that
/// a liveness read which is simply always empty would otherwise pass.
///
/// `max_parallel` is 1, so reaching 2 also says these items are exempt
/// from the launch cap rather than bounded by it.
#[tokio::test]
#[serial]
async fn a_jobs_two_group_over_eight_items_reaches_two_live_children_and_never_exceeds_two() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());
    setup_test_db(&work_dir);

    let blockers: Vec<Blocker> = (0..8).map(|i| Blocker::new(&work_dir, &format!("s{i}"))).collect();
    let tasks: Vec<Task> = blockers
        .iter()
        .enumerate()
        .map(|(i, blocker)| {
            let mut task = Task::new(
                format!("tail:s{i}"),
                Some("tail".to_string()),
                vec![],
                vec![],
                vec![],
                HashMap::new(),
                HashMap::new(),
                blocker.action(),
            );
            task.foreach_jobs = NonZeroUsize::new(2);
            task
        })
        .collect();
    let held: Vec<&Blocker> = blockers.iter().collect();

    let scheduler = scheduler_for(tasks, work_dir, 1).await?;
    let sampler = Sampler::watching(scheduler.clone());
    let running = run(&scheduler);

    // Release one started item at a time, whichever ones the group let
    // through: its two permits mean a started-but-unreleased item exists
    // until the last one is released, so this drives the whole group
    // without assuming which item took a permit first.
    let mut released: HashSet<usize> = HashSet::new();
    while released.len() < blockers.len() {
        let next =
            |released: &HashSet<usize>| (0..blockers.len()).find(|i| blockers[*i].started() && !released.contains(i));
        if !wait_for(|| next(&released).is_some()).await {
            release_all(&held);
            panic!("no started item was left to release; {} of 8 released", released.len());
        }
        let i = next(&released).expect("the barrier above found one");
        if !blockers[i].released().await {
            release_all(&held);
            panic!("item {i} never reached its barrier to be released");
        }
        released.insert(i);
    }

    let outcome = tokio::time::timeout(RUN_TIMEOUT, running).await;
    release_all(&held);
    outcome??.expect("every item must finish once released");

    let (max_live, max_candidates) = sampler.stop();
    assert_eq!(
        max_live, 2,
        "a jobs: 2 group must reach exactly 2 live children at tick time, and never more"
    );
    assert_eq!(
        max_candidates, 2,
        "every live item has a clock, so the candidate set must reach 2 as well"
    );
    Ok(())
}

/// **Phase 2 success criterion.** A completed task is gone from the
/// candidate set by the time its status line has been printed, while a
/// still-running sibling is still in it.
///
/// "By the time its status line has been printed" is exact rather than
/// approximate: the success arm calls `report_status_line` and only then
/// records the task's final status, so an observation gated on the status
/// map strictly follows the line. Both tasks are candidates first, so the
/// absence proved here is not the vacuous kind.
#[tokio::test]
#[serial]
async fn a_completed_task_leaves_the_candidate_set_and_a_running_sibling_stays() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());
    setup_test_db(&work_dir);

    let quick = Blocker::new(&work_dir, "quick");
    let slow = Blocker::new(&work_dir, "slow");
    let held = [&quick, &slow];
    let tasks = vec![blocking_task("quick", &quick), blocking_task("slow", &slow)];

    let scheduler = scheduler_for(tasks, work_dir, 2).await?;
    let running = run(&scheduler);

    if !wait_for_candidates(&scheduler, &["quick", "slow"]).await {
        release_all(&held);
        panic!("both tasks must be candidates before either of them completes");
    }

    if !quick.released().await {
        release_all(&held);
        panic!("the quick task never reached its barrier to be released");
    }
    if !wait_for_completion(&scheduler, "quick").await {
        release_all(&held);
        panic!("the released task never completed");
    }
    let names = candidate_names(&scheduler).await;

    if !slow.released().await {
        release_all(&held);
        panic!("the slow task never reached its barrier to be released");
    }
    let outcome = tokio::time::timeout(RUN_TIMEOUT, running).await;
    release_all(&held);
    outcome??.expect("both tasks must finish once released");

    assert_eq!(
        names,
        vec!["slow".to_string()],
        "a task whose status line has been printed is no longer a candidate, and its sibling still is"
    );
    Ok(())
}

/// **Phase 2 success criterion.** A cancelled run leaves the candidate set
/// empty.
///
/// This is the path a mirror of the registry would have missed:
/// `ActiveTasks::abort_all` clears the registry itself, and the aborted
/// bodies never run the deregistration that would have updated a copy, so
/// a mirror would name tasks SIGKILLed by Ctrl+C as still running for the
/// rest of the run. The clock entry deliberately survives - the empty read
/// comes from the liveness source of truth, not from anyone tidying up
/// clocks. The binary's Ctrl+C handler trips exactly this signal
/// (`install_stop_handler`, `app.rs`).
#[tokio::test]
#[serial]
async fn a_cancelled_run_leaves_no_candidates() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let work_dir = PathBuf::from(temp_dir.path());
    setup_test_db(&work_dir);

    let sleeper = Blocker::new(&work_dir, "sleeper");
    let tasks = vec![blocking_task("sleeper", &sleeper)];

    let scheduler = scheduler_for(tasks, work_dir, 2).await?;
    let cancel = scheduler.cancel_signal();
    let running = run(&scheduler);

    if !wait_for_candidates(&scheduler, &["sleeper"]).await {
        sleeper.release();
        panic!("the task must be a candidate before the run is cancelled");
    }

    cancel.cancel();
    let outcome = tokio::time::timeout(RUN_TIMEOUT, running).await;
    sleeper.release();
    let err = outcome??.expect_err("a cancelled run must not report success");
    assert!(
        format!("{err:#}").contains("cancelled"),
        "the error must say the run was cancelled; got {err:#}"
    );

    assert!(
        candidate_names(&scheduler).await.is_empty(),
        "a killed task must not be a heartbeat candidate"
    );
    assert!(
        scheduler.live_child_names().await.is_empty(),
        "abort_all clears the very registry the candidate set is read from"
    );
    assert!(
        scheduler.task_clocks().get("sleeper").is_some(),
        "the clock entry stays behind and is simply never read again"
    );
    Ok(())
}
