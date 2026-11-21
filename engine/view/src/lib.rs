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
//! `scene`: the points to draw, a sample of a scan's index for the view,
//! with the page's own classes where it has them. `camera`: the page's view
//! as the card's matrix. `gpu`: the card (the points coloured as the page
//! colours them, eye-dome lighting). `view`: the window, the thread that
//! draws in it and the one that reads the scans for it.
//!
//! 1.36: the page's look (its colour modes, class colours and layers,
//! intensity mixed in, point size, eye-dome lighting), the page's own
//! classes and removed points for a scan it has classified or edited, and
//! the detail for the view.
//!
//! 1.37: the page's shading (Leo, 2026-11-20 16:56: "i want you to
//! review/build the lighting and depth shading, detail, hollows and
//! faces"). `surface`: each point's lighting, relief, edge and hollow from
//! the shape of the surface it's on, worked out as the points are read;
//! `gpu`: those at the strengths the view asks for, and depth shading,
//! relief and face shading from the picture's depths. And the rest of
//! what kept a scan the page's to draw: `scene::Clip`, the limit boxes and
//! the slice (a view's detail is read by them, and the card tests each
//! point by them); `shapes`, the points isolated.

pub mod camera;
pub mod gpu;
pub mod scene;
pub mod shapes;
pub mod surface;
pub mod view;

pub use camera::{Affine, Camera, IDENTITY};
pub use gpu::{shading_reach, Cloud, Edl, Gpu, Look, Sender, Shading, BY_CLASS, BY_HEIGHT, BY_INTENSITY, BY_ONE, BY_OWN, CLEAR, EDL_DARKEST};
pub use scene::{drawn_box, ramp, sample, test_ring, wanted, Borrow, BorrowPoints, Clip, Keep, PointVertex, Points, Sight, Taken, CLIP_BOXES, RAMP};
pub use shapes::{Op, ShapeIn, Shapes};
pub use view::{Ask, ScanAsk, Stage, Stats, View};
