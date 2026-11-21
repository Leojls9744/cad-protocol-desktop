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
//!
//! 1.39: what was left of stage 1 (Leo, 2026-11-20 18:54: "what can we
//! finish of this"). `brush`: the brush's preview under the cursor, and
//! its strokes tested on the card until the scan's read with them, so a
//! press shows as it's painted; `view`: a scan's older reading kept on the
//! card while it's read again. `scene::Tint`: the page's own colouring for
//! the ones the engine hasn't (each point its nearest's colour). And 64
//! limit boxes on the card.
//!
//! 1.40: no change of a scan sends it back to the page (Leo, 2026-11-20
//! 20:37: "can we finalize this now or do we need to wait"). `view`: the
//! reading a scan had before its last change is kept on the card (an undo
//! is at once), a spare base with no points isolated is kept while some
//! are (Unisolate is at once), and an older reading may stand in till the
//! new one's base is in (`Older::TillBase`).
//!
//! 1.41: stage 2, linework drawn natively. `lines`: the page's own batches
//! of thin solid lines (the strips its card drew) on the engine's card,
//! drawn over the scans in the same frame, placed as the page places them.
//! 1.42: their linetypes (dashes, dots, dashes centred on each segment) and
//! a wide dashed polyline's filled dashes.

pub mod brush;
pub mod camera;
pub mod lines;
pub mod gpu;
pub mod scene;
pub mod shapes;
pub mod surface;
pub mod view;

pub use brush::{Preview, Stroke, StrokeOp, Strokes, PREVIEW_DABS, STROKES, STROKE_DABS};
pub use camera::{Affine, Camera, IDENTITY};
pub use gpu::{shading_reach, Cloud, Edl, Gpu, Look, Sender, Shading, ViewDraw, VIEWS, BY_CLASS, BY_HEIGHT, BY_INTENSITY, BY_ONE, BY_OWN, CLEAR, EDL_DARKEST, NO_STROKES};
pub use scene::{drawn_box, ramp, sample, test_ring, wanted, Borrow, BorrowPoints, Clip, Keep, PointVertex, Points, Sight, Taken, Tint, CLIP_BOXES, RAMP, TINT_NONE};
pub use lines::{covers, dash_here, period_of, LineItem, LineList, LineWorld, Lines, Strips, MAX_DASH, STRIP_FLOATS};
pub use shapes::{Op, ShapeIn, Shapes};
pub use view::{Ask, Older, ScanAsk, Stage, Stats, View};
