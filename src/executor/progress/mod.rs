//! otto's own terminal surface.
//!
//! Phase 2 of `docs/design/2026-09-16-live-progress-renderer.md` landed the
//! facade alone: one owner of otto's stdout and stderr, no renderer behind it
//! yet and no behaviour change. Phase 3 adds `mode`, the `--progress` /
//! `OTTO_PROGRESS` / `otto.progress` decision, computed once and stored - no
//! renderer reads it yet. The remaining phases add the terminal-ownership
//! handoff and the live region as further siblings of `facade`, all of them
//! reached only from inside it.

pub mod facade;
pub mod mode;

pub use facade::{Facade, Stream, facade};
pub use mode::{ProgressMode, ProgressSetting};
