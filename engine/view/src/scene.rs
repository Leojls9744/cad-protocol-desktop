//! What the engine draws: points, each a place and a colour, from a scan's
//! index or made up for the check.

use bytemuck::{Pod, Zeroable};
use cadp_engine_core::Index;

/// A point on the card: its place from its scan's middle (the file's own
/// units, as f32 near 0 so nothing's lost), and its colour.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PointVertex {
    pub pos: [f32; 3],
    pub color: [u8; 4],
}

/// Points to draw, from `centre` (the file's coordinates).
#[derive(Clone, Debug, Default)]
pub struct Points {
    pub centre: [f64; 3],
    /// How far any of them is from the middle, at most.
    pub radius: f64,
    pub data: Vec<PointVertex>,
}

/// Blue, green, yellow, red as `t` goes from 0 to 1: a scan with no colours by its heights.
fn ramp(t: f64) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let stops: [[f64; 3]; 5] = [
        [40.0, 80.0, 220.0],
        [40.0, 200.0, 220.0],
        [60.0, 200.0, 90.0],
        [235.0, 215.0, 60.0],
        [225.0, 70.0, 50.0],
    ];
    let at = t * 4.0;
    let i = (at.floor() as usize).min(3);
    let f = at - i as f64;
    let mix = |k: usize| (stops[i][k] + (stops[i + 1][k] - stops[i][k]) * f).round() as u8;
    [mix(0), mix(1), mix(2), 255]
}

/// An even sample of a scan, at most about `most` points: the same first
/// share of every cell (each cell's points are shuffled in the index, so a
/// first part of one is an even sample of it). Its own colours where it has
/// them, else by height.
pub fn sample(ix: &Index, most: usize) -> Points {
    let lo = ix.file_xyz(ix.min[0], ix.min[1], ix.min[2]);
    let hi = ix.file_xyz(ix.max[0], ix.max[1], ix.max[2]);
    let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let half = [(hi[0] - lo[0]) / 2.0, (hi[1] - lo[1]) / 2.0, (hi[2] - lo[2]) / 2.0];
    let radius = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
    let share = if ix.count == 0 { 0.0 } else { (most as f64 / ix.count as f64).min(1.0) };
    let tall = (hi[2] - lo[2]).max(1e-9);
    let mut data = Vec::with_capacity(((ix.count as f64 * share) as usize).saturating_add(ix.cells));
    for k in 0..ix.cells {
        let cell = ix.cell(k);
        if cell.count == 0 {
            continue;
        }
        let take = ((cell.count as f64 * share).ceil() as u64).clamp(1, cell.count as u64);
        for i in cell.start..cell.start + take {
            let p = ix.point(i);
            // Withheld points aren't shown anywhere.
            if p.flags & 4 != 0 {
                continue;
            }
            let at = ix.file_xyz(p.x, p.y, p.z);
            let color = if ix.has_rgb {
                let byte = |v: u16| if ix.wide { (v >> 8) as u8 } else { v.min(255) as u8 };
                [byte(p.rgb[0]), byte(p.rgb[1]), byte(p.rgb[2]), 255]
            } else {
                ramp((at[2] - lo[2]) / tall)
            };
            data.push(PointVertex {
                pos: [(at[0] - centre[0]) as f32, (at[1] - centre[1]) as f32, (at[2] - centre[2]) as f32],
                color,
            });
        }
    }
    Points { centre, radius, data }
}

/// The check's own picture: a ring of `n` points 100 across, coloured round
/// it, with a dot of white ones in the middle, about `centre`.
pub fn test_ring(n: usize, centre: [f64; 3]) -> Points {
    let mut data = Vec::with_capacity(n + n / 8);
    for i in 0..n {
        let a = i as f64 / n as f64 * std::f64::consts::TAU;
        // Three rings close together, so it's a band a few points wide.
        let r = 50.0 - (i % 3) as f64 * 0.6;
        data.push(PointVertex { pos: [(r * a.cos()) as f32, (r * a.sin()) as f32, 0.0], color: ramp(i as f64 / n as f64) });
    }
    for i in 0..n / 8 {
        let a = i as f64 * 2.399963;
        let r = 6.0 * (i as f64 / (n / 8).max(1) as f64).sqrt();
        data.push(PointVertex { pos: [(r * a.cos()) as f32, (r * a.sin()) as f32, 1.0], color: [255, 255, 255, 255] });
    }
    Points { centre, radius: 51.0, data }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vertex_is_16_bytes_as_the_card_reads_it() {
        assert_eq!(std::mem::size_of::<PointVertex>(), 16);
        let v = PointVertex { pos: [1.0, 2.0, 3.0], color: [4, 5, 6, 255] };
        let bytes: &[u8] = bytemuck::bytes_of(&v);
        assert_eq!(&bytes[12..], &[4, 5, 6, 255]);
    }

    #[test]
    fn heights_run_blue_to_red() {
        assert_eq!(ramp(0.0), [40, 80, 220, 255]);
        assert_eq!(ramp(1.0), [225, 70, 50, 255]);
        assert_eq!(ramp(-3.0), ramp(0.0));
        assert_eq!(ramp(0.5), [60, 200, 90, 255]);
    }

    #[test]
    fn the_checks_ring_is_round_its_middle() {
        let ring = test_ring(600, [10.0, 20.0, 30.0]);
        assert_eq!(ring.data.len(), 675);
        assert_eq!(ring.centre, [10.0, 20.0, 30.0]);
        let far = ring.data.iter().map(|v| (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1]).sqrt()).fold(0.0f32, f32::max);
        assert!(far <= 50.001 && far >= 49.9, "{far}");
    }
}
