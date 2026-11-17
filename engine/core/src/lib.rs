//! CAD Protocol's scan engine (desktop 1.19).
//!
//! A LAS or LAZ scan is read from disk once, on every core, and each of its
//! points filed by the cell of space it's in, in an index file kept on disk
//! (`index`). The page's snap then asks for the scan's points under the
//! cursor (`snap`): the engine walks the cells the cursor's line of sight
//! passes through, so nothing has to be drawn again after an orbit to know
//! what's under the cursor.
//!
//! Points are kept as the file has them (its whole-number coordinates, its
//! classes and flags); the page gives, with each question, how the file's
//! coordinates come into the drawing (its units, ground scale and
//! alignment, as one affine move), so one index serves every drawing.

pub mod index;
pub mod snap;

mod fx;

pub use index::{build_or_open, cache_name, Index, IndexError, Progress};
pub use snap::{snap, Candidate, ScanIn, SnapAnswer, SnapQuery};
