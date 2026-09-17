//! otto's own terminal surface.
//!
//! Phase 2 of `docs/design/2026-09-16-live-progress-renderer.md` landed the
//! facade alone: one owner of otto's stdout and stderr, no renderer behind it
//! yet and no behaviour change. Phase 3 adds `mode`, the `--progress` /
//! `OTTO_PROGRESS` / `otto.progress` decision, computed once and stored - no
//! renderer reads it yet. Phase 4 adds `ownership`, the handoff that stands
//! otto's writers down while a `tty:` task owns the terminal, reached only
//! through the facade. The live region is Phase 5's, as one more sibling
//! behind the same seam.

pub mod facade;
pub mod mode;
pub mod ownership;

pub use facade::{Facade, Stream, facade};
pub use mode::{ProgressMode, ProgressSetting};
pub use ownership::TerminalHandoff;
