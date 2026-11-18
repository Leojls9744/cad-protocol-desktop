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
//!
//! 1.20: a scan's points in a box of the drawing (`region`), for the
//! views' close-ups and the commands' regions, evenly thinned to what's
//! asked.
//!
//! 1.22: cloud to cloud (`c2c`): one scan lined up on another (point to
//! plane), and how far apart they are, on every core, as the page does it.
//!
//! 1.23: Geo W., the trained scan classifier (`net`), on every core, as
//! the page runs it.
//!
//! 1.24: E57 scans (`e57`), read as the page reads them, indexed as LAS
//! scans are; each point's setup kept, a one-setup scan asked of alone.
//!
//! 1.28: Delete Outside Limit Box (`crop`): every point of a scan cut to
//! the drawing's limit boxes from its index, written to a LAS or LAZ file;
//! renders (`render`): the page's ray tracer, rows on every core.

pub mod c2c;
pub mod crop;
pub mod e57;
pub mod index;
pub mod net;
pub mod region;
pub mod render;
pub mod snap;

mod fx;

pub use index::{build_fresh, build_or_open, cache_name, open_made, Index, IndexError, Progress};
pub use region::{region, RegionPoints, RegionQuery};
pub use snap::{snap, Candidate, ScanIn, SnapAnswer, SnapQuery};
