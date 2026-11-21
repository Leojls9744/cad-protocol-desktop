//! The card: a scan's points kept on it, and a frame drawn from them.
//!
//! Each point is a small square (or a disc) facing the eye, two triangles
//! made on the card from the one point kept, so its size is in pixels
//! whatever the zoom. The card colours it as the view asks (`Look`): by its
//! own colour, its class's (a table of 256, each with whether its layer
//! shows, so a class turned off or recoloured sends no points again), its
//! height, its intensity or one colour, with its intensity mixed in as the
//! page mixes it, then brightened or darkened by the shape of the surface
//! it's on (its lighting, relief, edges and hollows: surface.rs's four
//! bytes, at the strengths the view asks for). Nearer points cover further
//! ones (a depth buffer). A point the view's limit boxes or slice don't
//! show is put nowhere (`Clip`, tested on the card as the page's own card
//! tests its boxes, so a box dragged shows at once).
//!
//! A frame is two passes: the points into a picture of the engine's own
//! with their depths, then that picture onto the window shaded from the
//! depths as the page shades its own (cloud-gl.ts): depth shading (folds
//! darker), relief (lit from the upper left by the slope), face shading
//! (steep faces darker against Z) and eye-dome lighting, with the
//! background wherever there's no point. Drawn onto a window's surface, or
//! into a picture read back (the tests', and the build's check where
//! there's no window).

use crate::camera::{columns, Affine, Camera};
use crate::scene::{Clip, PointVertex, Points, CLIP_BOXES, RAMP};
use crate::surface;

/// What the page behind shows where nothing's drawn: the drawing's own background is the page's to give.
pub const CLEAR: [f64; 4] = [0.043, 0.063, 0.125, 1.0];

/// What a scan's points are coloured by.
pub const BY_OWN: u32 = 0;
pub const BY_CLASS: u32 = 1;
pub const BY_HEIGHT: u32 = 2;
pub const BY_INTENSITY: u32 = 3;
pub const BY_ONE: u32 = 4;

/// How a scan's points look, as the page has them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// BY_OWN (a scan with no colours: by height), BY_CLASS, BY_HEIGHT, BY_INTENSITY or BY_ONE.
    pub by: u32,
    /// A point's size, pixels.
    pub px: f64,
    /// Discs, not squares.
    pub round: bool,
    /// BY_ONE's colour, 0 to 1.
    pub one: [f32; 3],
    /// The heights (the drawing's z) BY_HEIGHT's colours run over, low and high.
    pub heights: [f64; 2],
    /// The intensities BY_INTENSITY's greys run over, low and high.
    pub intensity: [f32; 2],
    /// Intensity mixed into the colours: read on low to high, and its share, 0 to 1.
    pub mix: [f32; 3],
    /// Each class's colour (red, green, blue) and whether it shows (0: its layer is off).
    pub classes: [[u8; 4]; 256],
    /// The surface shading's strengths, 0 to 1: lighting, detail (relief and edges) and hollows.
    pub shade: [f32; 3],
    /// Fill gaps: each point as big as its spacing where it is (never under `px`), up to this many pixels; 0: off.
    pub fill: f64,
}

impl Look {
    /// Its own colours, every class showing, squares of 2 px.
    pub fn own() -> Look {
        Look { by: BY_OWN, px: 2.0, round: false, one: [1.0; 3], heights: [0.0, 1.0], intensity: [0.0, 65535.0], mix: [0.0, 1.0, 0.0], classes: [[175, 175, 175, 255]; 256], shade: [0.0; 3], fill: 0.0 }
    }
}

/// Eye-dome lighting as the page's: its strength (the setting over 35), how far off a pixel its eight neighbours are and the eye's distance (pixels), and the depth differences ignored (drawing units). No strength: none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edl {
    pub strength: f64,
    pub radius: f64,
    pub eye: f64,
    pub noise: f64,
}

/// The darkest eye-dome lighting makes a pixel (the page's EYE_DOME_DARKEST).
pub const EDL_DARKEST: f32 = 0.45;

/// The page's shading from the picture's depths, each as the page's own
/// factor (cloud-shading.ts): depth shading (shadingFactor: a pixel in a
/// fold darker), relief (reliefFactor: lit from the upper left by the
/// slope) and face shading (faceFactor: steep faces darker against Z). 0:
/// none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shading {
    pub depth: f64,
    pub relief: f64,
    pub faces: f64,
    /// How much of the background shows through the scans (the page's Show switches' faint: 0.7; 0: none).
    pub fade: f64,
}

/// The most one direction's fold counts, pixels of depth (the page's SHADING_CAP), a fold this small is the scanner's noise (SHADING_NOISE), and the share of a slope put down to the points' squares (SHADING_GRAIN).
pub const SHADING_CAP: f32 = 12.0;
pub const SHADING_NOISE: f32 = 0.5;
pub const SHADING_GRAIN: f32 = 1.0;
/// The farthest the shading looks for a neighbouring point, pixels (SHADING_REACH_MOST).
pub const SHADING_REACH_MOST: f64 = 24.0;
/// Relief: a slope steeper than this counts as this, and the darkest and brightest it makes a pixel.
pub const RELIEF_STEEPEST: f32 = 4.0;
pub const RELIEF_LOW: f32 = 0.45;
pub const RELIEF_HIGH: f32 = 1.35;

/// Where (pixels away) a neighbouring point is looked for, for points `px` pixels across: from a point's width to two more (the page's shadingReach).
pub fn shading_reach(px: f64) -> (f64, f64) {
    let first = (px + 0.5).floor().clamp(1.0, SHADING_REACH_MOST);
    (first, (first + (2.0 * first).max(2.0) - 1.0).min(SHADING_REACH_MOST))
}

fn points_shader() -> String {
    let stops: Vec<String> = RAMP.iter().map(|s| format!("vec3<f32>({:.1}, {:.1}, {:.1})", s[0], s[1], s[2])).collect();
    format!(
        r#"
struct U {{
  m: mat4x4<f32>,
  // xy: a pixel across and up the window (a point that many pixels wide reaches that far either side of its middle); z: discs; w: what it's coloured by.
  size: vec4<f32>,
  // x: a point's size, pixels; y: the points' average spacing, pixels (0: gaps not filled); z: the most a point grows to fill a gap, pixels.
  px: vec4<f32>,
  // A point's height over the low end of the colours, as a share of their span.
  zrow: vec4<f32>,
  // Intensity's low and one over its span, for greys; then for the mix.
  ints: vec4<f32>,
  // The one colour; w: the mix's share.
  one: vec4<f32>,
  // x: the points have colours of their own.
  flags: vec4<f32>,
  // The surface shading's strengths, 0 to 1: lighting, detail, hollows.
  lit: vec4<f32>,
  // The scan's own axes in the drawing (a point's place from its scan's middle, turned and scaled into the drawing).
  turn: array<vec4<f32>, 3>,
  // x: how many limit boxes are on; y: a slice is on (the last of the boxes' places).
  clips: vec4<f32>,
  // Each box: its middle from the scan's middle in the drawing, and its side (1: shows what's inside, -1: what's outside); its half sizes; its turn's cosine and sine.
  boxc: array<vec4<f32>, {slots}>,
  boxh: array<vec4<f32>, {slots}>,
  boxr: array<vec4<f32>, {slots}>,
  classes: array<vec4<f32>, 256>,
}};
@group(0) @binding(0) var<uniform> u: U;

struct VOut {{
  @builtin(position) pos: vec4<f32>,
  @location(0) color: vec3<f32>,
  @location(1) q: vec2<f32>,
}};

// Whether a box holds a place (from the scan's middle, in the drawing).
fn in_box(d: vec3<f32>, k: u32) -> bool {{
  let o = d - u.boxc[k].xyz;
  let r = u.boxr[k];
  let q = vec3<f32>(o.x * r.x - o.y * r.y, o.x * r.y + o.y * r.x, o.z);
  return all(abs(q) <= u.boxh[k].xyz);
}}

// Whether the limit boxes and the slice show a place (the page's inBoxes): out of every box that shows what's outside it, in one of those that show what's inside; and in the slice.
fn shown(d: vec3<f32>) -> bool {{
  if (u.clips.y > 0.5 && !in_box(d, {boxes}u)) {{
    return false;
  }}
  var any = false;
  var hit = false;
  let n = u32(u.clips.x);
  for (var k = 0u; k < n; k++) {{
    let inside = in_box(d, k);
    if (u.boxc[k].w < 0.0) {{
      if (inside) {{
        return false;
      }}
    }} else {{
      any = true;
      if (inside) {{
        hit = true;
      }}
    }}
  }}
  return !any || hit;
}}

fn ramp(t: f32) -> vec3<f32> {{
  var stops = array<vec3<f32>, 7>({stops});
  let f = clamp(t, 0.0, 1.0) * 6.0;
  let k = min(u32(floor(f)), 5u);
  let w = f - f32(k);
  return (stops[k] + (stops[k + 1u] - stops[k]) * w) / 255.0;
}}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @location(0) p: vec3<f32>, @location(1) c: vec4<u32>, @location(2) iv: vec2<u32>, @location(3) sh: vec4<u32>, @location(4) mo: vec4<u32>) -> VOut {{
  var corners = array<vec2<f32>, 4>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0));
  let q = corners[vi];
  let entry = u.classes[c.w];
  var o: VOut;
  o.q = q;
  o.color = vec3<f32>(0.0);
  // Its class's layer is off: nowhere.
  if (entry.a < 0.5) {{
    o.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    return o;
  }}
  // Not shown by the limit boxes or the slice: nowhere.
  if (u.clips.x + u.clips.y > 0.5 && !shown(vec3<f32>(dot(u.turn[0].xyz, p), dot(u.turn[1].xyz, p), dot(u.turn[2].xyz, p)))) {{
    o.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    return o;
  }}
  let centre = u.m * vec4<f32>(p, 1.0);
  // Its size: the size set, or (Fill gaps) as big as its spacing where it is, a whole pixel up, never under the size set nor over the most.
  var wide = u.px.x;
  if (u.px.y > 0.0 && mo.x > 0u) {{
    wide = clamp(max(wide, ceil(u.px.y * exp2((f32(mo.x) - 128.0) / 16.0))), 1.0, max(u.px.z, wide));
  }}
  o.pos = vec4<f32>(centre.xy + q * wide * u.size.xy, centre.z, 1.0);
  let by = u.size.w;
  let inten = f32(iv.x);
  var col = u.one.rgb;
  if (by < 0.5) {{
    if (u.flags.x > 0.5) {{
      col = vec3<f32>(c.xyz) / 255.0;
    }} else {{
      col = ramp(dot(u.zrow.xyz, p) + u.zrow.w);
    }}
  }} else if (by < 1.5) {{
    col = entry.rgb;
  }} else if (by < 2.5) {{
    col = ramp(dot(u.zrow.xyz, p) + u.zrow.w);
  }} else if (by < 3.5) {{
    let t = clamp((inten - u.ints.x) * u.ints.y, 0.0, 1.0);
    col = vec3<f32>((40.0 + 215.0 * t) / 255.0);
  }}
  // Its intensity mixed in, as the page mixes it: weakest 1 - 0.65 m of its colour, strongest 1 + 0.25 m.
  let m = u.one.w;
  if (m > 0.0) {{
    let t = clamp((inten - u.ints.z) * u.ints.w, 0.0, 1.0);
    col = min(col * (1.0 - m + m * (0.35 + 0.9 * t)), vec3<f32>(1.0));
  }}
  // Brighter or darker by the shape of its surface, its hue kept (the
  // page's detailFactor): lit by the way it faces, about level ground's
  // light; its relief off the broad surface; an edge a little darker; a
  // hollow darker by twice the sky it doesn't see. In linear light: the
  // colour's own curve with the factor's put through it (dark colours, on
  // the curve's straight foot, by the factor itself).
  if (u.lit.x + u.lit.y + u.lit.z > 0.0) {{
    let lit = clamp(1.0 + u.lit.x * (f32(sh.x) / 255.0 - {level:.7}), 0.75, 1.25);
    let relief = (f32(sh.y) - select(0.0, 256.0, sh.y > 127u)) / 127.0;
    let up = clamp(1.0 + u.lit.y * 0.55 * relief, 0.7, 1.3);
    let cut = select(1.0, clamp(1.0 - u.lit.y * 0.2 * f32(sh.z) / 255.0, 0.88, 1.0), sh.z > 0u);
    let hollow = select(1.0, max(0.5, 1.0 - u.lit.z * 2.0 * (1.0 - f32(sh.w) / 255.0)), u.lit.z > 0.0 && sh.w < 255u);
    let f = lit * up * cut * hollow;
    let g = pow(f, 0.41666667);
    col = clamp(select((col + 0.055) * g - 0.055, col * f, col <= vec3<f32>(0.04045)), vec3<f32>(0.0), vec3<f32>(1.0));
  }}
  o.color = col;
  return o;
}}

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {{
  if (u.size.z > 0.5 && dot(i.q, i.q) > 1.0) {{
    discard;
  }}
  return vec4<f32>(i.color, 1.0);
}}
"#,
        stops = stops.join(", "),
        level = surface::light_level(),
        slots = CLIP_BOXES + 1,
        boxes = CLIP_BOXES
    )
}

/// The picture onto the window: the background where there's no point, the
/// points' colours shaded from their depths where there is, as the page's
/// own shade pass (cloud-gl.ts SHADE_FRAGMENT), its numbers the page's:
///
/// - Depth shading: along the pixel's row and column, the nearest point
///   either side (from a point's width away to a few), each one's depth
///   less the pixel's, per pixel across. On a plane the two cancel; where
///   both stand nearer (a fold) their sum darkens it, less the scanner's
///   noise and, on a slope, the points' own squares.
/// - Relief (with no lighting from the surfaces): lit from the screen's
///   upper left by the slope through the pixel, steepened.
/// - Face shading: darker the steeper the surface stands against the
///   drawing's Z; of the two sides, the one rising least, so a crease
///   stays a crease.
/// - Eye-dome lighting: over the eight pixels a radius off, how much nearer
///   the eye each stands, as log2 of distances from an eye `eye` pixels in
///   front of this pixel's point, less the scanner's noise; then
///   exp(−strength × their sum), never darker than EDL_DARKEST.
fn shade_shader() -> String {
    format!(
        r#"
struct E {{
  // The background; a: how much of their strength the scans are drawn at (1: all; under: faint, the background showing through).
  clear: vec4<f32>,
  // Strength, radius (pixels), the eye's distance (pixels), the noise (pixels of depth).
  edl: vec4<f32>,
  // x: pixels of nearness to one of depth; y: the darkest eye-dome lighting makes; zw: the picture's size.
  more: vec4<f32>,
  // Depth shading's factor, relief's, face shading's, and the points' size (pixels).
  shade: vec4<f32>,
  // The first and last pixels off a neighbour's looked for; Z's share up the screen and toward the eye.
  reach: vec4<f32>,
}};
@group(0) @binding(0) var<uniform> e: E;
@group(0) @binding(1) var t_color: texture_2d<f32>;
@group(0) @binding(2) var t_depth: texture_depth_2d;

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {{
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return vec4<f32>(p[vi], 0.0, 1.0);
}}

// The nearest point stepping one way: whether there's one, its nearness (pixels of depth) and how far off (pixels).
struct Hit {{
  ok: bool,
  d: f32,
  at: f32,
}};

fn look(at: vec2<i32>, dir: vec2<i32>) -> Hit {{
  var h: Hit;
  h.ok = false;
  h.d = 0.0;
  h.at = 0.0;
  let size = vec2<i32>(e.more.zw);
  let last = i32(e.reach.y);
  for (var t = i32(e.reach.x); t <= last; t++) {{
    let p = at + dir * t;
    if (p.x < 0 || p.y < 0 || p.x >= size.x || p.y >= size.y) {{
      return h;
    }}
    let w = textureLoad(t_depth, p, 0);
    if (w < 1.0) {{
      h.ok = true;
      h.d = (0.5 - w) * e.more.x;
      h.at = f32(t);
      return h;
    }}
  }}
  return h;
}}

// How far the points either side stand nearer than the surface through this pixel would.
fn fold(d: f32, a: Hit, b: Hit) -> f32 {{
  if (!a.ok || !b.ok) {{
    return 0.0;
  }}
  let ga = (a.d - d) / a.at;
  let gb = (b.d - d) / b.at;
  let f = (ga + gb) * min(a.at, b.at) * 0.5;
  var slope = 0.0;
  if (ga * gb < 0.0) {{
    slope = min(abs(ga), abs(gb));
  }}
  return clamp(f - {noise:.2} - {grain:.2} * e.shade.w * slope, 0.0, {cap:.1});
}}

// The surface's rise from `a` (behind) to `b` (ahead), depth per pixel.
fn rise(d: f32, a: Hit, b: Hit) -> f32 {{
  if (a.ok && b.ok) {{
    return (b.d - a.d) / (a.at + b.at);
  }}
  if (b.ok) {{
    return (b.d - d) / b.at;
  }}
  if (a.ok) {{
    return (d - a.d) / a.at;
  }}
  return 0.0;
}}

// Face shading's rise: of the two sides, the one rising least, the scanner's noise off each.
fn face_rise(d: f32, a: Hit, b: Hit) -> f32 {{
  var ga = 0.0;
  if (a.ok) {{
    ga = sign(d - a.d) * max(abs(d - a.d) - e.edl.w, 0.0) / a.at;
  }}
  var gb = 0.0;
  if (b.ok) {{
    gb = sign(b.d - d) * max(abs(b.d - d) - e.edl.w, 0.0) / b.at;
  }}
  if (a.ok && b.ok) {{
    if (abs(ga) < abs(gb)) {{
      return ga;
    }}
    return gb;
  }}
  if (a.ok) {{
    return ga;
  }}
  return gb;
}}

@fragment
fn fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {{
  let at = vec2<i32>(frag.xy);
  let w = textureLoad(t_depth, at, 0);
  if (w >= 1.0) {{
    return vec4<f32>(e.clear.rgb, 1.0);
  }}
  let col = textureLoad(t_color, at, 0).rgb;
  let d = (0.5 - w) * e.more.x;
  var s = 1.0;
  if (e.shade.x > 0.0 || e.shade.y > 0.0 || e.shade.z > 0.0) {{
    // The nearest points left and right, and below and above (up the screen is ahead).
    let left = look(at, vec2<i32>(-1, 0));
    let right = look(at, vec2<i32>(1, 0));
    let below = look(at, vec2<i32>(0, 1));
    let above = look(at, vec2<i32>(0, -1));
    if (e.shade.x > 0.0) {{
      s = exp(-e.shade.x * (fold(d, left, right) + fold(d, below, above)));
    }}
    if (e.shade.y > 0.0) {{
      let n = vec3<f32>(
        -clamp(rise(d, left, right), -{steepest:.1}, {steepest:.1}) * e.shade.y,
        -clamp(rise(d, below, above), -{steepest:.1}, {steepest:.1}) * e.shade.y,
        1.0);
      let lit = dot(n, vec3<f32>(-0.5, 0.5, 0.70710678)) / length(n) / 0.70710678;
      s *= clamp(lit, {low:.2}, {high:.2});
    }}
    if (e.shade.z > 0.0) {{
      let gx = face_rise(d, left, right);
      let gy = face_rise(d, below, above);
      let level = abs(-gy * e.reach.z + e.reach.w) / length(vec3<f32>(gx, gy, 1.0));
      s *= 1.0 - e.shade.z * (1.0 - min(level, 1.0));
    }}
  }}
  if (e.edl.x > 0.0) {{
    let zc = e.edl.z;
    let least = zc / 256.0;
    let size = vec2<i32>(e.more.zw);
    var sum = 0.0;
    for (var i = 0; i < 8; i++) {{
      let a = f32(i) * 0.78539816;
      let p = at + vec2<i32>(floor(vec2<f32>(cos(a), sin(a)) * e.edl.y + 0.5));
      if (p.x < 0 || p.y < 0 || p.x >= size.x || p.y >= size.y) {{
        continue;
      }}
      let wn = textureLoad(t_depth, p, 0);
      if (wn >= 1.0) {{
        continue;
      }}
      let nearer = (0.5 - wn) * e.more.x - d - e.edl.w;
      if (nearer <= 0.0) {{
        continue;
      }}
      let zn = max(zc - nearer, least);
      sum += max(0.0, log2(zc) - log2(zn));
    }}
    s *= max(e.more.y, exp(-e.edl.x * sum));
  }}
  return vec4<f32>(mix(e.clear.rgb, min(col * s, vec3<f32>(1.0)), e.clear.a), 1.0);
}}
"#,
        noise = SHADING_NOISE,
        grain = SHADING_GRAIN,
        cap = SHADING_CAP,
        steepest = RELIEF_STEEPEST,
        low = RELIEF_LOW,
        high = RELIEF_HIGH,
    )
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    m: [[f32; 4]; 4],
    size: [f32; 4],
    px: [f32; 4],
    zrow: [f32; 4],
    ints: [f32; 4],
    one: [f32; 4],
    flags: [f32; 4],
    lit: [f32; 4],
    turn: [[f32; 4]; 3],
    clips: [f32; 4],
    boxc: [[f32; 4]; CLIP_BOXES + 1],
    boxh: [[f32; 4]; CLIP_BOXES + 1],
    boxr: [[f32; 4]; CLIP_BOXES + 1],
    classes: [[f32; 4]; 256],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShadeUniforms {
    clear: [f32; 4],
    edl: [f32; 4],
    more: [f32; 4],
    shade: [f32; 4],
    reach: [f32; 4],
}

/// The points' own picture's colours (the window's may be another).
const PICTURE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// A scan's points on the card.
pub struct Cloud {
    buffer: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
    pub count: u32,
    pub centre: [f64; 3],
    pub radius: f64,
    /// The points have colours of their own.
    pub colours: bool,
    /// Their shading from the shape of their surface is worked out.
    pub shaded: bool,
    /// How long that took, ms.
    pub shade_ms: f64,
    /// Their average spacing in the drawing (0: not measured).
    pub gap: f64,
}

/// What sends points to the card: the card's own, or a copy for another thread (a scan read while frames go on).
#[derive(Clone)]
pub struct Sender {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
}

impl Sender {
    /// A scan's points sent to the card.
    pub fn upload(&self, points: &Points) -> Cloud {
        // No more than one of the card's buffers holds (a scan's sample is far under it on any card of its own).
        let fits = (self.device.limits().max_buffer_size / std::mem::size_of::<PointVertex>() as u64).min(u32::MAX as u64) as usize;
        let data = &points.data[..points.data.len().min(fits)];
        let bytes: &[u8] = bytemuck::cast_slice(data);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cadp points"),
            size: (bytes.len().max(std::mem::size_of::<PointVertex>())) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // A part at a time, so no one write is past what the card's queue takes (each a whole number of points).
        const PART: usize = (64 << 20) / std::mem::size_of::<PointVertex>() * std::mem::size_of::<PointVertex>();
        let mut at = 0;
        while at < bytes.len() {
            let end = (at + PART).min(bytes.len());
            self.queue.write_buffer(&buffer, at as u64, &bytes[at..end]);
            at = end;
        }
        let uniforms = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cadp view"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cadp view"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        Cloud { buffer, uniforms, bind, count: data.len() as u32, centre: points.centre, radius: points.radius, colours: points.colours, shaded: points.shaded, shade_ms: points.shade_ms, gap: points.gap }
    }
}

/// The points' picture and depths for a frame of one size, and what reads them onto the window.
struct Frame {
    size: [u32; 2],
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

/// The card, ready to draw.
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,
    /// The card's name, and how it's reached (for the log).
    pub adapter: String,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    shade: wgpu::RenderPipeline,
    shade_layout: wgpu::BindGroupLayout,
    shade_uniforms: wgpu::Buffer,
    frame: Option<Frame>,
}

/// What's asked of the card: a real one where there is one.
fn adapter_options<'a>(surface: Option<&'a wgpu::Surface<'a>>) -> wgpu::RequestAdapterOptions<'a, 'a> {
    wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: surface,
    }
}

impl Gpu {
    /// For a window's surface (its format chosen from what the surface takes), or with none, for a picture of `format`.
    pub fn new(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, format: Option<wgpu::TextureFormat>) -> Result<Gpu, String> {
        let adapter = pollster::block_on(instance.request_adapter(&adapter_options(surface)))
            .or_else(|| {
                // No card of the computer's own: the processor's stand-in, if there's one.
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    force_fallback_adapter: true,
                    ..adapter_options(surface)
                }))
            })
            .ok_or_else(|| "no graphics card the engine can draw on".to_string())?;
        let info = adapter.get_info();
        let name = format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type);
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("cadp view"),
                required_features: wgpu::Features::empty(),
                // As little as it needs, but for the size of a scan's points and a wide screen.
                required_limits: wgpu::Limits {
                    max_buffer_size: adapter.limits().max_buffer_size,
                    ..wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits())
                },
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|e| format!("the graphics card wouldn't start: {e}"))?;
        let format = match (surface, format) {
            (Some(s), _) => {
                let caps = s.get_capabilities(&adapter);
                // As the page's colours are: the bytes given are the bytes shown.
                caps.formats
                    .iter()
                    .copied()
                    .find(|f| !f.is_srgb())
                    .or_else(|| caps.formats.first().copied())
                    .ok_or_else(|| "the window can't be drawn on by this graphics card".to_string())?
            }
            (None, Some(f)) => f,
            (None, None) => wgpu::TextureFormat::Rgba8Unorm,
        };
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cadp points"),
            source: wgpu::ShaderSource::Wgsl(points_shader().into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cadp points"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cadp points"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let attributes = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint8x4, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint16x2, offset: 16, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint8x4, offset: 20, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint8x4, offset: 24, shader_location: 4 },
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cadp points"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<PointVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format: PICTURE, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            // A point is a strip of two triangles, each point its own (four corners, not six: a third less for the card to place).
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let shade_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cadp shade"),
            source: wgpu::ShaderSource::Wgsl(shade_shader().into()),
        });
        let shade_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cadp shade"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
            ],
        });
        let shade_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cadp shade"),
            bind_group_layouts: &[&shade_layout],
            push_constant_ranges: &[],
        });
        let shade = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cadp shade"),
            layout: Some(&shade_pipeline_layout),
            vertex: wgpu::VertexState { module: &shade_shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shade_shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let shade_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cadp shade"),
            size: std::mem::size_of::<ShadeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Gpu { device, queue, format, adapter: name, pipeline, layout, shade, shade_layout, shade_uniforms, frame: None })
    }

    /// What sends points to this card, for this thread or another.
    pub fn sender(&self) -> Sender {
        Sender { device: self.device.clone(), queue: self.queue.clone(), layout: self.layout.clone() }
    }

    /// A scan's points sent to the card.
    pub fn upload(&self, points: &Points) -> Cloud {
        self.sender().upload(points)
    }

    /// The points' picture and depths, `size` pixels (made again when the window's another size).
    fn frame_for(&mut self, size: [u32; 2]) {
        if self.frame.as_ref().is_some_and(|f| f.size == size) {
            return;
        }
        let extent = wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 };
        let make = |label: &str, format: wgpu::TextureFormat| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let color = make("cadp points picture", PICTURE);
        let depth = make("cadp depth", DEPTH);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cadp shade"),
            layout: &self.shade_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.shade_uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&color) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&depth) },
            ],
        });
        self.frame = Some(Frame { size, color, depth, bind });
    }

    /// A frame: each scan through the view, as its look has it, into
    /// `target` (`size` pixels), shaded from its depths (`shading`) and
    /// with eye-dome lighting, only what the limit boxes and slice show
    /// (`clip`). With no view, the background alone.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        target: &wgpu::TextureView,
        size: [u32; 2],
        camera: Option<&Camera>,
        clouds: &[(&Cloud, Affine, &Look)],
        edl: &Edl,
        shading: &Shading,
        clip: &Clip,
        clear: [f64; 4],
    ) {
        self.frame_for(size);
        // How far the scans reach from the target along the line of sight: what the depth runs over.
        let mut reach = 1.0f64;
        if let Some(cam) = camera {
            for (cloud, affine, _) in clouds {
                let mid = [
                    affine[0] * cloud.centre[0] + affine[1] * cloud.centre[1] + affine[2] * cloud.centre[2] + affine[3] - cam.target[0],
                    affine[4] * cloud.centre[0] + affine[5] * cloud.centre[1] + affine[6] * cloud.centre[2] + affine[7] - cam.target[1],
                    affine[8] * cloud.centre[0] + affine[9] * cloud.centre[1] + affine[10] * cloud.centre[2] + affine[11] - cam.target[2],
                ];
                let along = mid[0] * cam.to_eye[0] + mid[1] * cam.to_eye[1] + mid[2] * cam.to_eye[2];
                // A move may stretch: its longest row says by how much at most.
                let stretch = (0..3)
                    .map(|r| (affine[r * 4] * affine[r * 4] + affine[r * 4 + 1] * affine[r * 4 + 1] + affine[r * 4 + 2] * affine[r * 4 + 2]).sqrt())
                    .fold(1.0f64, f64::max);
                reach = reach.max(along.abs() + cloud.radius * stretch * 1.8);
            }
            for (cloud, affine, look) in clouds {
                let m = cam.matrix(cloud.centre, affine, size, reach * 1.05);
                let half = look.px.max(1.0);
                // The drawing's height of a point from its scan's middle, over the colours' low end, as a share of their span.
                let span = (look.heights[1] - look.heights[0]).max(1e-9);
                let mid_z = affine[8] * cloud.centre[0] + affine[9] * cloud.centre[1] + affine[10] * cloud.centre[2] + affine[11];
                let inv = |lo: f32, hi: f32| if hi > lo { 1.0 / (hi - lo) } else { 0.0 };
                let mut classes = [[0.0f32; 4]; 256];
                for (k, c) in look.classes.iter().enumerate() {
                    classes[k] = [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, if c[3] > 0 { 1.0 } else { 0.0 }];
                }
                let mixing = look.mix[2] > 0.0 && look.mix[1] > look.mix[0] && look.by != BY_INTENSITY;
                // The limit boxes and the slice, from the scan's middle in the drawing (worked out in f64: a drawing far from 0 keeps its places).
                let mid = [
                    affine[0] * cloud.centre[0] + affine[1] * cloud.centre[1] + affine[2] * cloud.centre[2] + affine[3],
                    affine[4] * cloud.centre[0] + affine[5] * cloud.centre[1] + affine[6] * cloud.centre[2] + affine[7],
                    mid_z,
                ];
                let mut boxc = [[0.0f32; 4]; CLIP_BOXES + 1];
                let mut boxh = [[0.0f32; 4]; CLIP_BOXES + 1];
                let mut boxr = [[1.0f32, 0.0, 0.0, 0.0]; CLIP_BOXES + 1];
                let boxes = clip.boxes.len().min(CLIP_BOXES);
                for (k, b) in clip.boxes.iter().take(CLIP_BOXES).map(|b| (None, b)).chain(clip.slice.iter().map(|b| (Some(CLIP_BOXES), b))).enumerate().map(|(k, (at, b))| (at.unwrap_or(k), b)) {
                    for a in 0..3 {
                        boxc[k][a] = ((b.min[a] + b.max[a]) / 2.0 - mid[a]) as f32;
                        boxh[k][a] = ((b.max[a] - b.min[a]) / 2.0 + b.slack[a]) as f32;
                    }
                    boxc[k][3] = if b.outside { -1.0 } else { 1.0 };
                    if b.turned {
                        boxr[k] = [b.cos as f32, b.sin as f32, 0.0, 0.0];
                    }
                }
                let u = Uniforms {
                    m: columns(&m),
                    size: [(1.0 / size[0].max(1) as f64) as f32, (1.0 / size[1].max(1) as f64) as f32, if look.round { 1.0 } else { 0.0 }, look.by as f32],
                    // Fill gaps: the points' average spacing in pixels (each one's own is told by it), where it was measured.
                    px: [half as f32, if look.fill > 0.0 && cloud.gap > 0.0 { (cloud.gap / cam.units_per_pixel.max(1e-12)) as f32 } else { 0.0 }, look.fill.max(0.0) as f32, 0.0],
                    zrow: [(affine[8] / span) as f32, (affine[9] / span) as f32, (affine[10] / span) as f32, ((mid_z - look.heights[0]) / span) as f32],
                    ints: [look.intensity[0], inv(look.intensity[0], look.intensity[1]), look.mix[0], inv(look.mix[0], look.mix[1])],
                    one: [look.one[0], look.one[1], look.one[2], if mixing { look.mix[2].clamp(0.0, 1.0) } else { 0.0 }],
                    flags: [if cloud.colours { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
                    // Points whose shading isn't worked out are drawn as they are.
                    lit: if cloud.shaded { [look.shade[0].clamp(0.0, 1.0), look.shade[1].clamp(0.0, 1.0), look.shade[2].clamp(0.0, 1.0), 0.0] } else { [0.0; 4] },
                    turn: [
                        [affine[0] as f32, affine[1] as f32, affine[2] as f32, 0.0],
                        [affine[4] as f32, affine[5] as f32, affine[6] as f32, 0.0],
                        [affine[8] as f32, affine[9] as f32, affine[10] as f32, 0.0],
                    ],
                    clips: [boxes as f32, if clip.slice.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0],
                    boxc,
                    boxh,
                    boxr,
                    classes,
                };
                self.queue.write_buffer(&cloud.uniforms, 0, bytemuck::bytes_of(&u));
            }
        }
        // Depth's 0 to 1 is `reach × 1.05` drawing units each side of the target: as pixels of nearness.
        let upp = camera.map(|c| c.units_per_pixel).filter(|u| *u > 0.0).unwrap_or(1.0);
        let on = edl.strength > 0.0 && edl.eye > 0.0;
        // The biggest point: where a neighbouring point is looked for.
        let px = clouds.iter().map(|(_, _, look)| (look.px + 0.5).floor()).fold(1.0f64, f64::max);
        let (first, last) = shading_reach(px);
        let strength = |v: f64| if v.is_finite() && v > 0.0 { v as f32 } else { 0.0 };
        let shade = ShadeUniforms {
            clear: [clear[0] as f32, clear[1] as f32, clear[2] as f32, (1.0 - if shading.fade.is_finite() { shading.fade.clamp(0.0, 1.0) } else { 0.0 }) as f32],
            edl: [if on { edl.strength as f32 } else { 0.0 }, edl.radius.max(1.0) as f32, edl.eye as f32, (edl.noise.max(0.0) / upp) as f32],
            more: [(2.0 * reach * 1.05 / upp) as f32, EDL_DARKEST, size[0] as f32, size[1] as f32],
            shade: [strength(shading.depth), strength(shading.relief), strength(shading.faces), px as f32],
            // The drawing's Z up the screen and toward the eye (the view's up and line of sight, their z).
            reach: [first as f32, last as f32, camera.map(|c| c.up[2]).unwrap_or(0.0) as f32, camera.map(|c| c.to_eye[2]).unwrap_or(1.0) as f32],
        };
        self.queue.write_buffer(&self.shade_uniforms, 0, bytemuck::bytes_of(&shade));
        let frame = self.frame.as_ref().expect("the frame's picture");
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cadp frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cadp points"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.color,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &frame.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some(cam) = camera {
                if let Some([x, y, w, h]) = cam.scissor(size) {
                    pass.set_scissor_rect(x, y, w, h);
                    pass.set_pipeline(&self.pipeline);
                    for (cloud, _, _) in clouds {
                        if cloud.count == 0 {
                            continue;
                        }
                        pass.set_bind_group(0, &cloud.bind, &[]);
                        pass.set_vertex_buffer(0, cloud.buffer.slice(..));
                        pass.draw(0..4, 0..cloud.count);
                    }
                }
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cadp shade"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0], g: clear[1], b: clear[2], a: clear[3] }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shade);
            pass.set_bind_group(0, &frame.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
    }

    /// A frame drawn into a picture of the engine's own, and read back: its
    /// pixels, red green blue and how solid, row by row from the top.
    pub fn picture(
        &mut self,
        size: [u32; 2],
        camera: Option<&Camera>,
        clouds: &[(&Cloud, Affine, &Look)],
        edl: &Edl,
        shading: &Shading,
        clip: &Clip,
        clear: [f64; 4],
    ) -> Result<Vec<u8>, String> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cadp picture"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.draw(&view, size, camera, clouds, edl, shading, clip, clear);
        // Rows are sent padded to 256 bytes.
        let row = (size[0] * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cadp picture"),
            size: (row * size[1]) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cadp picture") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size[1]) },
            },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().map_err(|e| e.to_string())?.map_err(|e| format!("the picture couldn't be read back: {e}"))?;
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((size[0] * size[1] * 4) as usize);
        for y in 0..size[1] {
            let at = (y * row) as usize;
            out.extend_from_slice(&data[at..at + (size[0] * 4) as usize]);
        }
        Ok(out)
    }
}
