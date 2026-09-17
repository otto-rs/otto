//! otto's own terminal surface.
//!
//! Phase 2 of `docs/design/2026-09-16-live-progress-renderer.md` lands the
//! facade alone: one owner of otto's stdout and stderr, no renderer behind it
//! yet and no behaviour change. The later phases add the mode decision, the
//! terminal-ownership handoff, and the live region as siblings of `facade`,
//! all of them reached only from inside it.

pub mod facade;

pub use facade::{Facade, Stream, facade};
