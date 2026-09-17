//! otto's own terminal surface.
//!
//! Phase 2 of `docs/design/2026-09-16-live-progress-renderer.md` landed the
//! facade alone: one owner of otto's stdout and stderr, no renderer behind it
//! yet and no behaviour change. Phase 3 adds `mode`, the `--progress` /
//! `OTTO_PROGRESS` / `otto.progress` decision, computed once and stored - no
//! renderer reads it yet. Phase 4 adds `ownership`, the handoff that stands
//! otto's writers down while a `tty:` task owns the terminal, reached only
//! through the facade. Phase 5 adds `region`, the live rows themselves, plus
//! `duration`, the one format a row and a completion line both use so the two
//! cannot disagree in presentation after this phase made them agree in
//! measurement.

pub mod duration;
pub mod facade;
pub mod mode;
pub mod ownership;
pub mod region;

pub use duration::format_task_duration;
pub use facade::{Facade, Stream, facade, install_region};
pub use mode::{ProgressMode, ProgressSetting};
pub use ownership::TerminalHandoff;
pub use region::Region;
