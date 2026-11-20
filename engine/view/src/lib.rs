//! The scan engine drawing its scans itself (desktop 1.35; Leo, 2026-11-20
//! 12:08: "i want to transfer everything to the desktop app"; stage 1's
//! Trial B).
//!
//! Until now the engine read a scan and handed its points to the page,
//! which drew them. Here the engine keeps the points on the graphics card
//! and draws them in a window of its own, a child of the app's frame
//! (desktop 1.34's layer trial: the frame's inside is the layer, the app's
//! page over it with a see-through drawing area), following the view the
//! page gives it.
//!
//! `scene`: the points to draw, an even sample of a scan's index. `camera`:
//! the page's view as the card's matrix. `gpu`: the card. `view`: the
//! window and the thread that draws in it.

pub mod camera;
pub mod gpu;
pub mod scene;
pub mod view;

pub use camera::{Affine, Camera, IDENTITY};
pub use gpu::{Cloud, Gpu, CLEAR};
pub use scene::{sample, test_ring, Keep, PointVertex, Points};
pub use view::{Ask, ScanAsk, Stats, View};
