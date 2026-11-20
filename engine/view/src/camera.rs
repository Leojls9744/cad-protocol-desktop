//! The page's view, as the engine draws by it.
//!
//! The page says where its drawing area looks (the middle of it, a place in
//! the drawing), which ways are right and up on screen and toward the eye
//! (three unit lengths), how many drawing units a pixel is, and where the
//! drawing area is in the window (pixels of the window's inside). Nothing
//! is nearer or smaller with distance: the page's views are parallel.

/// The view as the page gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub target: [f64; 3],
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub to_eye: [f64; 3],
    /// Drawing units a pixel (of the window's own pixels).
    pub units_per_pixel: f64,
    /// The drawing area in the window: left, top, width, height, pixels.
    pub rect: [f64; 4],
}

/// A scan's file coordinates into the drawing: three rows of four (as the
/// page gives every question of a scan).
pub type Affine = [f64; 12];

pub const IDENTITY: Affine = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl Camera {
    /// From a point's place from its scan's middle (`centre`, the file's
    /// coordinates) to where the card puts it: across and up from −1 to 1
    /// over the whole window (`surface`, its pixels), and how far back from
    /// 0 (nearest) to 1, `depth` drawing units each side of the target
    /// along the line of sight. Worked out in f64, so a drawing far from 0
    /// keeps its places; rows first (m[row][col]).
    pub fn matrix(&self, centre: [f64; 3], affine: &Affine, surface: [u32; 2], depth: f64) -> [[f64; 4]; 4] {
        // The drawing's axes, as the scan's own axes come into it.
        let col = |c: usize| [affine[c], affine[4 + c], affine[8 + c]];
        // The scan's middle in the drawing, from the target.
        let mid = [
            affine[0] * centre[0] + affine[1] * centre[1] + affine[2] * centre[2] + affine[3] - self.target[0],
            affine[4] * centre[0] + affine[5] * centre[1] + affine[6] * centre[2] + affine[7] - self.target[1],
            affine[8] * centre[0] + affine[9] * centre[1] + affine[10] * centre[2] + affine[11] - self.target[2],
        ];
        let w = surface[0].max(1) as f64;
        let h = surface[1].max(1) as f64;
        let upp = if self.units_per_pixel > 0.0 { self.units_per_pixel } else { 1.0 };
        // Pixels across and down from the window's top-left: the drawing
        // area's middle is where the target shows.
        let cx = self.rect[0] + self.rect[2] / 2.0;
        let cy = self.rect[1] + self.rect[3] / 2.0;
        let d = if depth > 0.0 { depth } else { 1.0 };
        let mut m = [[0.0; 4]; 4];
        for c in 0..3 {
            m[0][c] = dot(col(c), self.right) / upp * 2.0 / w;
            m[1][c] = dot(col(c), self.up) / upp * 2.0 / h;
            m[2][c] = -dot(col(c), self.to_eye) / (2.0 * d);
        }
        m[0][3] = (dot(mid, self.right) / upp + cx) * 2.0 / w - 1.0;
        m[1][3] = 1.0 - (cy - dot(mid, self.up) / upp) * 2.0 / h;
        m[2][3] = 0.5 - dot(mid, self.to_eye) / (2.0 * d);
        m[3][3] = 1.0;
        m
    }

    /// The drawing area as the card's scissor: left, top, width, height,
    /// whole pixels inside the window; None when there's none of it.
    pub fn scissor(&self, surface: [u32; 2]) -> Option<[u32; 4]> {
        let x0 = self.rect[0].floor().max(0.0);
        let y0 = self.rect[1].floor().max(0.0);
        let x1 = (self.rect[0] + self.rect[2]).ceil().min(surface[0] as f64);
        let y1 = (self.rect[1] + self.rect[3]).ceil().min(surface[1] as f64);
        if !(x1 > x0 && y1 > y0) {
            return None;
        }
        Some([x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32])
    }
}

/// A matrix of f64 rows as the card's columns of f32.
pub fn columns(m: &[[f64; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for (r, row) in m.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            out[c][r] = *v as f32;
        }
    }
    out
}

/// Where a place from the scan's middle lands: across, up, back.
pub fn through(m: &[[f64; 4]; 4], p: [f64; 3]) -> [f64; 3] {
    let row = |r: usize| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3];
    [row(0), row(1), row(2)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plan view: east right, north up, the eye overhead.
    fn plan(target: [f64; 3], upp: f64, rect: [f64; 4]) -> Camera {
        Camera { target, right: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], to_eye: [0.0, 0.0, 1.0], units_per_pixel: upp, rect }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn the_target_shows_at_the_drawing_areas_middle() {
        // A window 1000 × 800; the drawing area its right three quarters, under a ribbon 200 high.
        let cam = plan([6_000_000.0, 2_000_000.0, 100.0], 0.5, [250.0, 200.0, 750.0, 600.0]);
        let m = cam.matrix([6_000_000.0, 2_000_000.0, 100.0], &IDENTITY, [1000, 800], 500.0);
        let at = through(&m, [0.0, 0.0, 0.0]);
        // Pixel 625, 500: across 625 / 500 − 1, up 1 − 500 / 400.
        assert!(close(at[0], 0.25) && close(at[1], -0.25) && close(at[2], 0.5), "{at:?}");
        // 50 units east and 30 north at half a unit a pixel: 100 px right, 60 px up.
        let east = through(&m, [50.0, 30.0, 0.0]);
        assert!(close(east[0], 0.25 + 100.0 / 500.0) && close(east[1], -0.25 + 60.0 / 400.0), "{east:?}");
        // Higher is nearer the eye from overhead: less far back.
        let high = through(&m, [0.0, 0.0, 100.0]);
        assert!(close(high[2], 0.4), "{high:?}");
    }

    #[test]
    fn a_scan_far_from_zero_keeps_its_places() {
        let centre = [6_286_133.5, 2_099_733.25, 335.0];
        let cam = plan([centre[0] + 12.0, centre[1] - 7.0, 0.0], 0.01, [0.0, 0.0, 2000.0, 1000.0]);
        let m = cam.matrix(centre, &IDENTITY, [2000, 1000], 1000.0);
        // A point 0.25 units east of the middle: 12 − 0.25 units left of the target, at 100 px a unit.
        let at = through(&m, [0.25, 0.0, 0.0]);
        assert!(close(at[0], -(12.0 - 0.25) * 100.0 * 2.0 / 2000.0), "{at:?}");
        // As f32 for the card, a pixel is still told apart (2 / 2000 across).
        let f = columns(&m);
        let x = f[0][0] * 0.25 + f[3][0];
        assert!((x as f64 - at[0]).abs() < 0.2 * 2.0 / 2000.0, "{x} {}", at[0]);
    }

    #[test]
    fn a_moved_scan_is_drawn_where_the_drawing_has_it() {
        // Turned a quarter left about the up axis, then 100 east: the file's x becomes north.
        let affine: Affine = [0.0, -1.0, 0.0, 100.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let cam = plan([100.0, 0.0, 0.0], 1.0, [0.0, 0.0, 400.0, 400.0]);
        let m = cam.matrix([0.0, 0.0, 0.0], &affine, [400, 400], 100.0);
        let at = through(&m, [10.0, 0.0, 0.0]);
        // 10 north of the target: 10 px up of 200.
        assert!(close(at[0], 0.0) && close(at[1], 10.0 / 200.0), "{at:?}");
    }

    #[test]
    fn the_scissor_is_the_drawing_area_inside_the_window() {
        let cam = plan([0.0; 3], 1.0, [250.4, 199.6, 750.0, 700.0]);
        assert_eq!(cam.scissor([1000, 800]), Some([250, 199, 750, 601]));
        let none = plan([0.0; 3], 1.0, [1200.0, 0.0, 10.0, 10.0]);
        assert_eq!(none.scissor([1000, 800]), None);
    }

    #[test]
    fn the_cards_matrix_is_by_columns() {
        let mut m = [[0.0; 4]; 4];
        m[0][3] = 7.0;
        m[2][1] = 3.0;
        let f = columns(&m);
        assert_eq!(f[3][0], 7.0);
        assert_eq!(f[1][2], 3.0);
    }
}
