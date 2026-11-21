//! Linework on the card (1.41: stage 2 of moving the app's work into the
//! desktop app, "linework drawn natively").
//!
//! The page still works out each object's lines; what it used to give its
//! own card (gl-lines.ts: a batch of segments a colour, packed as strips,
//! nine floats a segment) it gives the engine once, and the engine draws
//! them over the scans in the same frame, placed by what the page says with
//! each view. So the lines and the scans under them are one picture: they
//! can't part while the view turns.
//!
//! The strips are the page's own, as its `segmentsOf` and `worldStrips`
//! pack them, and the shader here is its full lines' shader (FULL_VERTEX,
//! FULL_FRAGMENT) for solid lines: each segment a strip a pixel wider than
//! the line each side, each pixel coloured by how much of it the line
//! covers, round past a soft end and cut at a smooth joint. A batch is
//! either in the view's plane (its ends across and up from the painter's
//! origin, plane units) or in the drawing's own places (from the draw's
//! world origin: the card works out across and up from the view's right
//! and up, so a turning view sends nothing again).
//!
//! 1.42 (build 2): linetypes. A batch's dashes are the card's, as the
//! page's shader draws them (its DASHES: a pattern of dash and gap lengths
//! in CSS pixels, a dot a length of −1 drawn round; running on along a run,
//! or centred on each CAD segment so it starts and ends on a dash at its
//! vertices), and a wide dashed polyline's dashes are filled as triangles
//! beneath the lines. `dash_here` is the shader's pattern worked on the
//! processor, as the page's own `dashHereOnCpu` is.
//!
//! 1.44 (build 4): the page's filled shapes (a hatch's fill, a tapered
//! polyline's widths). An `area` is its outline's loops as triangle fans
//! (three corners of three floats each: across and up the plane, and 0),
//! filled by the page's rule through the stencil: each fan's triangles
//! flip (even-odd) or count (nonzero) the pixels they cover, then the same
//! triangles are drawn in the shape's colour where the stencil says
//! inside, clearing it as they go. So a shape with holes, or one that
//! folds over itself, fills as the page's canvas fills it, with nothing
//! worked out by the page but the fans. They're drawn before everything
//! else of the page's: under its lines, as the page paints them.
//!
//! 1.43 (build 3): text. A batch may be `thin` (text's letters: a
//! hairline, the view's `thin` half-width) and `lettered` (a turning
//! view's letters: each strip a run's anchor in the drawing, its two ends
//! along and above the baseline, and the baseline's angle; the card turns
//! each letter to face the screen along its baseline, as the page's does).

use crate::gpu::VIEWS;
use std::collections::HashMap;
use std::sync::Arc;

/// Floats a segment takes (the page's STRIP_FLOATS): its two ends (three
/// each), whether each end is soft (1) or cut (0), and its span (dashes:
/// not read here).
pub const STRIP_FLOATS: usize = 9;

/// The most dash and gap lengths a pattern has (the page's MAX_DASH).
pub const MAX_DASH: usize = 12;

/// A batch's segments as the page packed them, under the number the page
/// names them by; or, for a batch that's filled, its triangles' corners
/// (three floats a corner: across and up the plane, and 0).
pub struct Strips {
    pub id: u64,
    pub floats: Vec<f32>,
}

impl Strips {
    /// How many segments.
    pub fn count(&self) -> u32 {
        (self.floats.len() / STRIP_FLOATS) as u32
    }

    /// How many corners of whole triangles, read as a filled batch's.
    pub fn corners(&self) -> u32 {
        let n = (self.floats.len() / 3) as u32;
        n - n % 3
    }
}

/// One batch of a list: its segments, its colour (red, green, blue 0 to 1
/// and how solid) and whether its ends are places in the drawing; its
/// dashes (dash and gap lengths in CSS pixels as the page's card takes
/// them, a dot −1, MAX_DASH at most; none: solid); or `fill`: its floats
/// are triangles, filled beneath the lines.
/// `thin`: drawn a hairline wide (text's letters). `lettered`: a world
/// batch whose strips are letters' (the page's letterStrip).
#[derive(Clone)]
pub struct LineItem {
    pub strips: Arc<Strips>,
    pub rgba: [f32; 4],
    pub world: bool,
    pub dash: Vec<f32>,
    pub fill: bool,
    pub thin: bool,
    pub lettered: bool,
    /// A filled shape of the page's (1.44): its floats are its loops as triangle fans, filled even-odd, or `nonzero`.
    pub area: bool,
    pub nonzero: bool,
}

impl LineItem {
    /// A solid batch of lines.
    pub fn solid(strips: Arc<Strips>, rgba: [f32; 4], world: bool) -> LineItem {
        LineItem { strips, rgba, world, dash: Vec::new(), fill: false, thin: false, lettered: false, area: false, nonzero: false }
    }
}

/// The stencil the areas are worked out in.
pub(crate) const STENCIL: wgpu::TextureFormat = wgpu::TextureFormat::Stencil8;
/// How many samples a pixel the filled shapes' own picture has (1.45): their edges come out smooth, as the page's canvas smooths its own.
pub(crate) const SAMPLES: u32 = 4;

/// Where `along` a line falls in a dash pattern, as the page's shader
/// works it out (gl-lines.ts dashHere; its dashHereOnCpu is the same on
/// the page): 1 in a gap or 0 on a dash, and how far along the nearest
/// dot's middle is (−1: none). `along` and `span`: CSS pixels from the CAD
/// segment's start and its length, or from the run's start with `span` 0
/// (the pattern running on). `dash`: the lengths, a dot −1.
pub fn dash_here(along: f64, span: f64, dash: &[f32]) -> (f64, f64) {
    let count = dash.len().min(MAX_DASH);
    if count == 0 {
        return (0.0, -1.0);
    }
    let period = period_of(dash) as f64;
    // GLSL's mod: never below 0.
    let modulo = |x: f64, y: f64| x - y * (x / y).floor();
    let pattern_at = |d: f64, lo: f64, hi: f64| -> (f64, f64, f64) {
        let (mut edge, mut gap, mut found, mut nearest, mut dots) = (0.0f64, 0.0f64, false, 0.0f64, 0.0f64);
        for (i, len) in dash.iter().take(count).enumerate() {
            let len = *len as f64;
            if len < 0.0 {
                let mut o = edge - d;
                o -= period * (o / period + 0.5).floor();
                if o <= lo {
                    o += period;
                } else if o >= hi {
                    o -= period;
                }
                if o > lo && o < hi && (dots < 0.5 || o.abs() < nearest.abs()) {
                    nearest = o;
                    dots = 1.0;
                }
            } else {
                edge += len;
                if !found && d < edge {
                    found = true;
                    gap = (i % 2) as f64;
                }
            }
        }
        (gap, nearest, dots)
    };
    if span <= 0.0 {
        let run = pattern_at(modulo(along, period), -period, period);
        return (run.0, if run.2 > 0.5 { run.1.abs() } else { -1.0 });
    }
    let d0 = (dash[0] as f64).max(0.0);
    let n = ((span - d0) / period).floor();
    if n < 1.0 {
        return (0.0, -1.0);
    }
    let ends = (span - n * period + d0) * 0.5;
    let dot_ends = dash[0] < 0.0;
    let pat = pattern_at(modulo(along - ends + d0, period), ends + 0.01 - along, span - ends - 0.01 - along);
    let mut away = if pat.2 > 0.5 { pat.1.abs() } else { -1.0 };
    if dot_ends {
        let vertex = along.abs().min((span - along).abs());
        away = if away < 0.0 { vertex } else { away.min(vertex) };
    }
    if along <= ends || along >= span - ends {
        return (if dot_ends { 1.0 } else { 0.0 }, away);
    }
    (pat.0, away)
}

/// A pattern's period: its lengths added up (a dot has none), never under a millionth (the page's periodOf).
pub fn period_of(dash: &[f32]) -> f32 {
    dash.iter().take(MAX_DASH).map(|d| d.max(0.0)).sum::<f32>().max(1e-6)
}

/// The batches a view draws, in the order they're drawn.
pub struct LineList {
    pub key: String,
    pub items: Vec<LineItem>,
}

/// How the drawing's own places come onto the screen: the view's right and
/// up, and where the world batches' origin is in the view (the page's CSS
/// pixels from its corner).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineWorld {
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub e: f64,
    pub f: f64,
}

/// A view's lines this frame: which, and where the page puts them (its
/// GlPlace: x = e + r·scale, y = f − u·scale, CSS pixels from the view's
/// corner), how wide (half the width, the screen's pixels) and how many of
/// the screen's pixels a CSS pixel is.
#[derive(Clone)]
pub struct Lines {
    pub list: Arc<LineList>,
    pub scale: f64,
    pub e: f64,
    pub f: f64,
    pub world: Option<LineWorld>,
    pub half: f64,
    pub ratio: f64,
    /// A dot's radius, the screen's pixels (the page's: the line's half-width, and at least its SCREEN_DOT_PX across).
    pub dot: f64,
    /// Half a thin batch's width (text's letters: a hairline), the screen's pixels; 0: as the others.
    pub thin: f64,
}

/// The lines' shader: the page's full lines' (FULL_VERTEX, FULL_FRAGMENT with its DASHES), and its fills'.
fn shader() -> &'static str {
    r#"
struct V {
    // The plane's scale, where its origin is (CSS pixels), and the screen's pixels a CSS pixel.
    place: vec4<f32>,
    // The window's size and the view's corner in it, the screen's pixels.
    size: vec4<f32>,
    // The view's right, and half a line's width.
    right: vec4<f32>,
    // The view's up, and a dot's radius (the screen's pixels).
    up: vec4<f32>,
    // Where the world's origin is (CSS pixels).
    wplace: vec4<f32>,
};
struct B {
    color: vec4<f32>,
    // x: its ends are places in the drawing (1), or it's letters there (2). y: how many dash lengths. z: the pattern's period. w: half its width, where it isn't the view's (0).
    kind: vec4<f32>,
    // The dash and gap lengths, CSS pixels (a dot: −1).
    dash: array<vec4<f32>, 3>,
};
@group(0) @binding(0) var<uniform> v: V;
@group(1) @binding(0) var<uniform> b: B;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    // Across the line from its middle, along it from its start, and its length: the screen's pixels.
    @location(0) rel: vec3<f32>,
    @location(1) caps: vec2<f32>,
    // How far along its CAD segment (or its run), and that segment's length (0: the pattern runs on): CSS pixels.
    @location(2) along: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @location(0) a: vec3<f32>, @location(1) bb: vec3<f32>, @location(2) caps: vec2<f32>, @location(3) span: f32) -> VOut {
    // Two triangles: (start or end, one side or the other).
    var corners = array<vec2<f32>, 6>(vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
    let corner = corners[vi];
    let world = b.kind.x > 0.5;
    var ra = a.xy;
    var rb = bb.xy;
    var at = v.place.yz;
    var cap = caps;
    if (b.kind.x > 1.5) {
        // A letter's segment: its run's anchor placed by the view, its ends along and above the baseline as the view
        // looks at it (the letter facing the screen, sized by how long the baseline looks).
        let base = vec2<f32>(dot(a, v.right.xyz), dot(a, v.up.xyz));
        let ct = cos(span);
        let st = sin(span);
        let along = vec2<f32>(ct * v.right.x + st * v.right.y, ct * v.up.x + st * v.up.y);
        let above = vec2<f32>(-along.y, along.x);
        ra = base + bb.x * along + bb.y * above;
        rb = base + bb.z * along + caps.x * above;
        let second = floor(caps.y / 2.0 + 0.25);
        cap = vec2<f32>(caps.y - 2.0 * second, second);
        at = v.wplace.xy;
    } else if (world) {
        ra = vec2<f32>(dot(a, v.right.xyz), dot(a, v.up.xyz));
        rb = vec2<f32>(dot(bb, v.right.xyz), dot(bb, v.up.xyz));
        at = v.wplace.xy;
    }
    var half = v.right.w;
    if (b.kind.w > 0.0) {
        half = b.kind.w;
    }
    let pa = vec2<f32>(at.x + ra.x * v.place.x, at.y - ra.y * v.place.x) * v.place.w;
    let pb = vec2<f32>(at.x + rb.x * v.place.x, at.y - rb.y * v.place.x) * v.place.w;
    let d = pb - pa;
    let len = length(d);
    var dir = vec2<f32>(1.0, 0.0);
    if (len > 1e-6) {
        dir = d / len;
    }
    let nrm = vec2<f32>(-dir.y, dir.x);
    let reach = half + 1.0;
    var ext = cap.y * reach;
    if (corner.x < 0.5) {
        ext = cap.x * reach;
    }
    let t = corner.x * len + (corner.x * 2.0 - 1.0) * ext;
    let s = corner.y * reach;
    let p = pa + dir * t + nrm * s + v.size.zw;
    var out: VOut;
    out.pos = vec4<f32>(p.x / v.size.x * 2.0 - 1.0, 1.0 - p.y / v.size.y * 2.0, 0.0, 1.0);
    out.rel = vec3<f32>(s, t, len);
    out.caps = cap;
    var frac = 0.0;
    if (len > 1e-6) {
        frac = t / len;
    }
    out.along = vec2<f32>(0.0, 0.0);
    if (!world) {
        out.along = vec2<f32>((a.z + (bb.z - a.z) * frac) * v.place.x, span * v.place.x);
    }
    return out;
}

fn dash_at(i: i32) -> f32 {
    return b.dash[i / 4][i % 4];
}

// Where `d` along the pattern falls: x 1 in a gap (0 on a dash), y how far the nearest dot's middle is (between lo and hi), z whether there's one.
fn pattern_at(d: f32, lo: f32, hi: f32) -> vec3<f32> {
    let n = i32(b.kind.y);
    let period = b.kind.z;
    var edge = 0.0;
    var gap = 0.0;
    var found = false;
    var nearest = 0.0;
    var dots = 0.0;
    for (var i = 0; i < 12; i++) {
        if (i >= n) {
            break;
        }
        let len = dash_at(i);
        if (len < 0.0) {
            var o = edge - d;
            o -= period * floor(o / period + 0.5);
            if (o <= lo) {
                o += period;
            } else if (o >= hi) {
                o -= period;
            }
            if (o > lo && o < hi && (dots < 0.5 || abs(o) < abs(nearest))) {
                nearest = o;
                dots = 1.0;
            }
        } else {
            edge += len;
            if (!found && d < edge) {
                found = true;
                gap = f32(i % 2);
            }
        }
    }
    return vec3<f32>(gap, nearest, dots);
}

// The page's dashHere: x 1 in a gap, y how far along the nearest dot's middle is (−1: none).
fn dash_here(along: f32, span: f32) -> vec2<f32> {
    let n = i32(b.kind.y);
    if (n <= 0) {
        return vec2<f32>(0.0, -1.0);
    }
    let period = b.kind.z;
    if (span <= 0.0) {
        let run = pattern_at(along - period * floor(along / period), -period, period);
        var away = -1.0;
        if (run.z > 0.5) {
            away = abs(run.y);
        }
        return vec2<f32>(run.x, away);
    }
    let d0 = max(dash_at(0), 0.0);
    let k = floor((span - d0) / period);
    if (k < 1.0) {
        return vec2<f32>(0.0, -1.0);
    }
    let ends = (span - k * period + d0) * 0.5;
    let dot_ends = dash_at(0) < 0.0;
    let x = along - ends + d0;
    let pat = pattern_at(x - period * floor(x / period), ends + 0.01 - along, span - ends - 0.01 - along);
    var away = -1.0;
    if (pat.z > 0.5) {
        away = abs(pat.y);
    }
    if (dot_ends) {
        let vertex = min(abs(along), abs(span - along));
        if (away < 0.0) {
            away = vertex;
        } else {
            away = min(away, vertex);
        }
    }
    if (along <= ends || along >= span - ends) {
        var end_gap = 0.0;
        if (dot_ends) {
            end_gap = 1.0;
        }
        return vec2<f32>(end_gap, away);
    }
    return vec2<f32>(pat.x, away);
}

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    let here = dash_here(i.along.x, i.along.y);
    // How much of this pixel the line covers: across it along the segment, round past a soft end, cut where the next segment goes on.
    let before = -i.rel.y;
    let after = i.rel.y - i.rel.z;
    var cs = step(before, 0.0);
    if (i.caps.x > 0.5) {
        cs = 1.0;
    }
    var ce = step(after, 0.0);
    if (i.caps.y > 0.5) {
        ce = 1.0;
    }
    let past = max(max(before, after), 0.0);
    var half = v.right.w;
    if (b.kind.w > 0.0) {
        half = b.kind.w;
    }
    var line = clamp(half + 0.5 - length(vec2<f32>(i.rel.x, past)), 0.0, 1.0);
    if (here.x > 0.5) {
        line = 0.0;
    }
    // A dot: round, its radius from its middle along the line and across it alike.
    var spot = 0.0;
    if (here.y >= 0.0) {
        spot = clamp(v.up.w + 0.5 - length(vec2<f32>(here.y * v.place.w, i.rel.x)), 0.0, 1.0);
    }
    let alpha = max(line, spot) * min(cs, ce) * b.color.a;
    if (alpha <= 0.0) {
        discard;
    }
    return vec4<f32>(b.color.rgb * alpha, alpha);
}

// A filled batch (a wide dashed polyline's dashes): its triangles' corners across and up the plane.
@vertex
fn vs_fill(@location(0) a: vec3<f32>) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(v.place.y + a.x * v.place.x, v.place.z - a.y * v.place.x) * v.place.w + v.size.zw;
    return vec4<f32>(p.x / v.size.x * 2.0 - 1.0, 1.0 - p.y / v.size.y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_fill() -> @location(0) vec4<f32> {
    return vec4<f32>(b.color.rgb * b.color.a, b.color.a);
}
"#
}

/// What puts the filled shapes' picture over the window (1.45): each pixel of it as it is (colour times opacity), blended over what's there.
fn put_shader() -> &'static str {
    r#"
@group(0) @binding(0) var layer: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(layer, vec2<i32>(at.xy), 0);
}
"#
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ViewUniforms {
    place: [f32; 4],
    size: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    wplace: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BatchUniforms {
    color: [f32; 4],
    kind: [f32; 4],
    dash: [[f32; 4]; 3],
}

/// A view's batches' colours on the card: each at its own place in one buffer, for the list they're of.
struct Listed {
    key: String,
    /// The list itself, by where it's kept (two lists may share a key only by the page's mistake: told apart by the list).
    of: usize,
    buffer: wgpu::Buffer,
    bind: wgpu::BindGroup,
    room: usize,
    /// The thin batches' half-width they were written with.
    thin: f32,
}

/// The filled shapes' own picture (1.45), the window's size: drawn with `SAMPLES` samples a pixel through a stencil
/// as fine, read down to one a pixel (`one`), and put over the scans before the lines. Made when a list first has a
/// filled shape; let go when none has had one for a while.
pub(crate) struct Smooth {
    size: [u32; 2],
    pub(crate) many: wgpu::TextureView,
    pub(crate) stencil: wgpu::TextureView,
    pub(crate) one: wgpu::TextureView,
    bind: wgpu::BindGroup,
    /// Frames since a list had a filled shape.
    idle: u32,
}

/// How many frames with no filled shape before their picture's let go (it's made again in a moment when one comes).
const SMOOTH_IDLE: u32 = 600;

/// The lines on the card: their strips (kept while the page keeps them), and what draws them.
pub(crate) struct LineCard {
    pipeline: wgpu::RenderPipeline,
    /// Filled batches' (triangles), into the filled shapes' picture.
    fills: wgpu::RenderPipeline,
    /// That picture put over the window, and what it's read by.
    put: wgpu::RenderPipeline,
    put_layout: wgpu::BindGroupLayout,
    smooth: Option<Smooth>,
    format: wgpu::TextureFormat,
    /// The areas': their fans marked in the stencil (even-odd, nonzero), then covered in their colour.
    mark_evenodd: wgpu::RenderPipeline,
    mark_nonzero: wgpu::RenderPipeline,
    cover: wgpu::RenderPipeline,
    batch_layout: wgpu::BindGroupLayout,
    views: [wgpu::Buffer; VIEWS],
    view_binds: [wgpu::BindGroup; VIEWS],
    listed: [Option<Listed>; VIEWS],
    /// Each batch's strips on the card, by the page's number for them.
    strips: HashMap<u64, (Arc<Strips>, wgpu::Buffer)>,
    /// How far apart the batches' colours are in their buffer (the card's own step).
    step: u64,
}

/// Where a list is kept: what tells it from another.
fn at(list: &Arc<LineList>) -> usize {
    Arc::as_ptr(list) as usize
}

impl LineCard {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> LineCard {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("cadp lines"), source: wgpu::ShaderSource::Wgsl(shader().into()) });
        let uniform = |dynamic: bool, size: u64| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: dynamic, min_binding_size: wgpu::BufferSize::new(size) },
            count: None,
        };
        let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("cadp lines view"), entries: &[uniform(false, std::mem::size_of::<ViewUniforms>() as u64)] });
        let batch_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("cadp lines batch"), entries: &[uniform(true, std::mem::size_of::<BatchUniforms>() as u64)] });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("cadp lines"), bind_group_layouts: &[&view_layout, &batch_layout], push_constant_ranges: &[] });
        let attributes = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 32, shader_location: 3 },
        ];
        // As the page's card blends its lines over what's under them: their colour times how much of the pixel they cover.
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cadp lines"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                buffers: &[wgpu::VertexBufferLayout { array_stride: (STRIP_FLOATS * 4) as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &attributes }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let fills = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cadp line fills"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_fill"),
                buffers: &[wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attributes[..1] }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_fill"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            // In the filled shapes' pass (1.45), which has a stencil: not looked at, not written.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: STENCIL,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
            multiview: None,
            cache: None,
        });
        // The areas: a fan's triangles through the stencil. Marking writes no colour; covering writes it where the
        // stencil isn't 0, and puts it back to 0 (so a pixel under several of the fan's triangles is coloured once).
        let face = |pass_op: wgpu::StencilOperation, compare: wgpu::CompareFunction| wgpu::StencilFaceState { compare, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op };
        let stenciled = |label: &str, front: wgpu::StencilFaceState, back: wgpu::StencilFaceState, colour: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_fill"),
                    buffers: &[wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attributes[..1] }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_fill"),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: if colour { wgpu::ColorWrites::ALL } else { wgpu::ColorWrites::empty() } })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: STENCIL,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::Always,
                    stencil: wgpu::StencilState { front, back, read_mask: 0xff, write_mask: 0xff },
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
                multiview: None,
                cache: None,
            })
        };
        let always = wgpu::CompareFunction::Always;
        let mark_evenodd = stenciled("cadp areas even-odd", face(wgpu::StencilOperation::Invert, always), face(wgpu::StencilOperation::Invert, always), false);
        let mark_nonzero = stenciled("cadp areas nonzero", face(wgpu::StencilOperation::IncrementWrap, always), face(wgpu::StencilOperation::DecrementWrap, always), false);
        let inside = face(wgpu::StencilOperation::Zero, wgpu::CompareFunction::NotEqual);
        let cover = stenciled("cadp areas cover", inside, inside, true);
        // The filled shapes' picture over the window: read a pixel at a time, blended as the lines are.
        let put_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("cadp fills over"), source: wgpu::ShaderSource::Wgsl(put_shader().into()) });
        let put_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cadp fills over"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            }],
        });
        let put_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("cadp fills over"), bind_group_layouts: &[&put_layout], push_constant_ranges: &[] });
        let put = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cadp fills over"),
            layout: Some(&put_pipeline_layout),
            vertex: wgpu::VertexState { module: &put_module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &put_module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let views: [wgpu::Buffer; VIEWS] = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some("cadp lines view"), size: std::mem::size_of::<ViewUniforms>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
        });
        let view_binds: [wgpu::BindGroup; VIEWS] = std::array::from_fn(|k| {
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("cadp lines view"), layout: &view_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: views[k].as_entire_binding() }] })
        });
        let step = (device.limits().min_uniform_buffer_offset_alignment as u64).max(std::mem::size_of::<BatchUniforms>() as u64);
        LineCard { pipeline, fills, put, put_layout, smooth: None, format, mark_evenodd, mark_nonzero, cover, batch_layout, views, view_binds, listed: std::array::from_fn(|_| None), strips: HashMap::new(), step }
    }

    /// A view's lines made ready for its frame: strips the card hasn't
    /// sent to it, the list's colours where it's another list, and where
    /// the page puts them. `size`: the window's pixels; `corner`: the
    /// view's corner in it.
    pub(crate) fn set(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, slot: usize, size: [u32; 2], corner: [f64; 2], lines: &Lines) {
        use wgpu::util::DeviceExt;
        if slot >= VIEWS {
            return;
        }
        for item in &lines.list.items {
            let s = &item.strips;
            let used = if item.fill || item.area { s.corners() as usize * 3 } else { s.count() as usize * STRIP_FLOATS };
            if used == 0 {
                continue;
            }
            // The same number with other strips (the page's own doing): those now.
            if self.strips.get(&s.id).is_some_and(|(had, _)| Arc::ptr_eq(had, s)) {
                continue;
            }
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("cadp line strips"), contents: bytemuck::cast_slice(&s.floats[..used]), usage: wgpu::BufferUsages::VERTEX });
            self.strips.insert(s.id, (s.clone(), buffer));
        }
        let n = lines.list.items.len();
        let thin = if lines.thin > 0.0 { lines.thin as f32 } else { 0.0 };
        let same = self.listed[slot].as_ref().is_some_and(|l| l.key == lines.list.key && l.of == at(&lines.list) && l.thin == thin);
        if !same && n > 0 {
            let room = self.listed[slot].as_ref().map_or(0, |l| l.room);
            let (buffer, bind, room) = match self.listed[slot].take() {
                Some(l) if room >= n => (l.buffer, l.bind, l.room),
                _ => {
                    let room = n.next_power_of_two().max(16);
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("cadp line colours"), size: room as u64 * self.step, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("cadp line colours"),
                        layout: &self.batch_layout,
                        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &buffer, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<BatchUniforms>() as u64) }) }],
                    });
                    (buffer, bind, room)
                }
            };
            let mut bytes = vec![0u8; n * self.step as usize];
            for (k, item) in lines.list.items.iter().enumerate() {
                let n = item.dash.len().min(MAX_DASH);
                let mut dash = [[0.0f32; 4]; 3];
                for (i, d) in item.dash.iter().take(n).enumerate() {
                    dash[i / 4][i % 4] = *d;
                }
                let kind = if item.lettered { 2.0 } else if item.world { 1.0 } else { 0.0 };
                // (A thin batch's half-width is the view's own number: the same for every view that draws the list.)
                let u = BatchUniforms { color: item.rgba, kind: [kind, n as f32, period_of(&item.dash), if item.thin { thin } else { 0.0 }], dash };
                let at = k * self.step as usize;
                bytes[at..at + std::mem::size_of::<BatchUniforms>()].copy_from_slice(bytemuck::bytes_of(&u));
            }
            queue.write_buffer(&buffer, 0, &bytes);
            self.listed[slot] = Some(Listed { key: lines.list.key.clone(), of: at(&lines.list), buffer, bind, room, thin });
        }
        let w = lines.world.unwrap_or(LineWorld { right: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], e: 0.0, f: 0.0 });
        let u = ViewUniforms {
            place: [lines.scale as f32, lines.e as f32, lines.f as f32, lines.ratio as f32],
            size: [size[0].max(1) as f32, size[1].max(1) as f32, corner[0] as f32, corner[1] as f32],
            right: [w.right[0] as f32, w.right[1] as f32, w.right[2] as f32, lines.half as f32],
            up: [w.up[0] as f32, w.up[1] as f32, w.up[2] as f32, lines.dot.max(lines.half) as f32],
            wplace: [w.e as f32, w.f as f32, 0.0, 0.0],
        };
        queue.write_buffer(&self.views[slot], 0, bytemuck::bytes_of(&u));
    }

    /// The strips no list of the page's holds any more, off the card
    /// (`now`: those this frame draws, kept whatever). The page's lists
    /// hold theirs while a view may name them, so lines left to the page
    /// for a moment (something selected) are still here when they're the
    /// engine's again.
    pub(crate) fn tidy(&mut self, now: &[&Lines]) {
        if self.strips.is_empty() {
            return;
        }
        self.strips.retain(|id, (s, _)| Arc::strong_count(s) > 1 || now.iter().any(|l| l.list.items.iter().any(|i| i.strips.id == *id)));
    }

    /// How many strips are on the card (tests).
    pub(crate) fn held(&self) -> usize {
        self.strips.len()
    }

    /// The filled shapes' picture made ready for a frame of `size` pixels with one in it (`wanted`), or counted idle
    /// and let go in time. What a frame with one draws into.
    pub(crate) fn smooth_for(&mut self, device: &wgpu::Device, size: [u32; 2], wanted: bool) {
        if !wanted {
            if let Some(s) = self.smooth.as_mut() {
                s.idle += 1;
                if s.idle > SMOOTH_IDLE {
                    self.smooth = None;
                }
            }
            return;
        }
        if let Some(s) = self.smooth.as_mut() {
            if s.size == size {
                s.idle = 0;
                return;
            }
        }
        let extent = wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 };
        let make = |label: &str, format: wgpu::TextureFormat, samples: u32, usage: wgpu::TextureUsages| {
            device
                .create_texture(&wgpu::TextureDescriptor { label: Some(label), size: extent, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let many = make("cadp fills", self.format, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let stencil = make("cadp fills stencil", STENCIL, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let one = make("cadp fills read down", self.format, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("cadp fills over"), layout: &self.put_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&one) }] });
        self.smooth = Some(Smooth { size, many, stencil, one, bind, idle: 0 });
    }

    /// The filled shapes' picture, when there's one.
    pub(crate) fn smooth(&self) -> Option<&Smooth> {
        self.smooth.as_ref()
    }

    /// Whether a view's lines have anything filled (a shape of the page's, or a filled batch).
    pub(crate) fn fills_in(lines: &Lines) -> bool {
        lines.list.items.iter().any(|i| i.area || i.fill)
    }

    /// A view's filled shapes and filled batches drawn into its part of their own picture, in the list's order (a
    /// pass with the stencil and `SAMPLES` samples; its scissor is the view's): a shape's fans marked in the
    /// stencil, then covered in its colour; a filled batch's triangles as they are.
    pub(crate) fn draw_under(&self, pass: &mut wgpu::RenderPass<'_>, slot: usize, lines: &Lines) {
        let Some(listed) = self.listed.get(slot).and_then(Option::as_ref) else { return };
        if listed.of != at(&lines.list) || listed.key != lines.list.key {
            return;
        }
        pass.set_bind_group(0, &self.view_binds[slot], &[]);
        pass.set_stencil_reference(0);
        for (k, item) in lines.list.items.iter().enumerate() {
            if !item.area && !item.fill {
                continue;
            }
            let Some((s, buffer)) = self.strips.get(&item.strips.id) else { continue };
            if s.corners() == 0 || !Arc::ptr_eq(s, &item.strips) {
                continue;
            }
            pass.set_bind_group(1, &listed.bind, &[(k as u64 * self.step) as u32]);
            pass.set_vertex_buffer(0, buffer.slice(..));
            if item.area {
                pass.set_pipeline(if item.nonzero { &self.mark_nonzero } else { &self.mark_evenodd });
                pass.draw(0..s.corners(), 0..1);
                pass.set_pipeline(&self.cover);
            } else {
                pass.set_pipeline(&self.fills);
            }
            pass.draw(0..s.corners(), 0..1);
        }
    }

    /// The filled shapes' picture put over the window (a pass with no scissor set: where nothing was filled it's clear).
    pub(crate) fn put_under(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(s) = self.smooth.as_ref() else { return };
        pass.set_pipeline(&self.put);
        pass.set_bind_group(0, &s.bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// A view's lines drawn into its part of the window (the pass's scissor is the view's): each batch in turn, its segments as strips.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, slot: usize, lines: &Lines) {
        let Some(listed) = self.listed.get(slot).and_then(Option::as_ref) else { return };
        if listed.of != at(&lines.list) || listed.key != lines.list.key {
            return;
        }
        // (The filled batches and shapes are in their own picture, already put under these: draw_under, put_under.)
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.view_binds[slot], &[]);
        for (k, item) in lines.list.items.iter().enumerate() {
            // In the drawing's own places: none without the view's right and up.
            if item.fill || item.area || ((item.world || item.lettered) && lines.world.is_none()) {
                continue;
            }
            let Some((s, buffer)) = self.strips.get(&item.strips.id) else { continue };
            if s.count() == 0 || !Arc::ptr_eq(s, &item.strips) {
                continue;
            }
            pass.set_bind_group(1, &listed.bind, &[(k as u64 * self.step) as u32]);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..6, 0..s.count());
        }
    }
}

/// One of a list's lines near a place in its view (1.47; stage 3): which
/// of the list's items, the page's number for its strips, which of its
/// segments (a filled batch's triangle; 0 for a filled shape), how far from
/// the place (CSS pixels; 0 inside a filled one), how far along the segment
/// (0 its start, 1 its end), and whether the place is INSIDE a filled shape
/// or batch, not near a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineHit {
    pub item: usize,
    pub id: u64,
    pub segment: u32,
    pub dist: f64,
    pub along: f64,
    pub inside: bool,
}

/// The lines of a list within `reach` CSS pixels of a place in a view (x
/// across and y down from the view's corner, CSS pixels), nearest first,
/// then the filled shapes and batches the place is inside, in the list's
/// order: `most` at most of each. The view puts the list as a frame's
/// `Lines` does (`scale`, `e`, `f`; `world` for the batches in the
/// drawing's own places, which without it aren't drawn and aren't found).
/// What's found is what's drawn: a dashed line through its gaps too (it's
/// one line to pick), text's letters at rest as their segments; a turning
/// view's letters aren't looked at. Worked out here on the processor, from
/// the strips as the page gave them: no card.
#[allow(clippy::too_many_arguments)]
pub fn pick(list: &LineList, scale: f64, e: f64, f: f64, world: Option<&LineWorld>, x: f64, y: f64, reach: f64, most: usize) -> Vec<LineHit> {
    let mut near: Vec<LineHit> = Vec::new();
    let mut inside: Vec<LineHit> = Vec::new();
    if most == 0 || !(reach >= 0.0) {
        return near;
    }
    let reach2 = reach * reach;
    // The worst kept so far once `most` are: a segment farther isn't looked at closer.
    let mut bar = reach2;
    for (k, item) in list.items.iter().enumerate() {
        let v = &item.strips.floats;
        if item.area || item.fill {
            // Its triangles, across and up the plane: how many the place is in (and which way round each runs).
            let n = item.strips.corners() as usize;
            let mut count = 0i32;
            let mut wound = 0i32;
            let mut first = None;
            for t in 0..n / 3 {
                let c = |i: usize| (e + v[(t * 3 + i) * 3] as f64 * scale, f - v[(t * 3 + i) * 3 + 1] as f64 * scale);
                let (a, b, cc) = (c(0), c(1), c(2));
                let d1 = (x - b.0) * (a.1 - b.1) - (a.0 - b.0) * (y - b.1);
                let d2 = (x - cc.0) * (b.1 - cc.1) - (b.0 - cc.0) * (y - cc.1);
                let d3 = (x - a.0) * (cc.1 - a.1) - (cc.0 - a.0) * (y - a.1);
                let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
                let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
                if neg && pos {
                    continue;
                }
                // (A triangle with no area holds nothing.)
                let twice = (b.0 - a.0) * (cc.1 - a.1) - (cc.0 - a.0) * (b.1 - a.1);
                if twice == 0.0 {
                    continue;
                }
                count += 1;
                wound += if twice > 0.0 { 1 } else { -1 };
                if first.is_none() {
                    first = Some(t as u32);
                }
            }
            let within = if !item.area { count > 0 } else if item.nonzero { wound != 0 } else { count % 2 == 1 };
            if within && inside.len() < most {
                inside.push(LineHit { item: k, id: item.strips.id, segment: if item.area { 0 } else { first.unwrap_or(0) }, dist: 0.0, along: 0.0, inside: true });
            }
            continue;
        }
        if item.lettered || (item.world && world.is_none()) {
            continue;
        }
        let place = |p: &[f32]| -> (f64, f64) {
            match (item.world, world) {
                (true, Some(w)) => {
                    let (px, py, pz) = (p[0] as f64, p[1] as f64, p[2] as f64);
                    let r = px * w.right[0] + py * w.right[1] + pz * w.right[2];
                    let u = px * w.up[0] + py * w.up[1] + pz * w.up[2];
                    (w.e + r * scale, w.f - u * scale)
                }
                _ => (e + p[0] as f64 * scale, f - p[1] as f64 * scale),
            }
        };
        for (n, strip) in v.chunks_exact(STRIP_FLOATS).enumerate() {
            let a = place(&strip[0..3]);
            let b = place(&strip[3..6]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 { (((x - a.0) * dx + (y - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let (qx, qy) = (a.0 + dx * t - x, a.1 + dy * t - y);
            let d2 = qx * qx + qy * qy;
            if !(d2 <= bar) {
                continue;
            }
            near.push(LineHit { item: k, id: item.strips.id, segment: n as u32, dist: d2.sqrt(), along: t, inside: false });
            // Kept few: the nearest `most`, the rest let go as they pile up.
            if near.len() >= most.saturating_mul(4).max(64) {
                near.sort_by(|p, q| p.dist.total_cmp(&q.dist));
                near.truncate(most);
                if near.len() == most {
                    bar = near[most - 1].dist * near[most - 1].dist;
                }
            }
        }
    }
    near.sort_by(|p, q| p.dist.total_cmp(&q.dist));
    near.truncate(most);
    near.extend(inside);
    near
}

/// How much of a pixel a solid line covers, as the shader works it out
/// (tests check the card's pixels against it): `across` from the line's
/// middle and `past` its end (0 beside it), the screen's pixels, for a line
/// `half` its width.
pub fn covers(half: f64, across: f64, past: f64) -> f64 {
    (half + 0.5 - (across * across + past.max(0.0) * past.max(0.0)).sqrt()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_of(a: [f32; 3], b: [f32; 3]) -> [f32; 9] {
        [a[0], a[1], a[2], b[0], b[1], b[2], 1.0, 1.0, 0.0]
    }

    fn lines_of(id: u64, strips: &[[f32; 9]], world: bool) -> LineItem {
        LineItem::solid(Arc::new(Strips { id, floats: strips.iter().flatten().copied().collect() }), [1.0; 4], world)
    }

    fn shape_of(id: u64, loops: &[&[[f32; 2]]], nonzero: bool) -> LineItem {
        let mut floats = Vec::new();
        for l in loops {
            for k in 1..l.len().saturating_sub(1) {
                for p in [l[0], l[k], l[k + 1]] {
                    floats.extend_from_slice(&[p[0], p[1], 0.0]);
                }
            }
        }
        LineItem { area: true, nonzero, ..LineItem::solid(Arc::new(Strips { id, floats }), [1.0; 4], false) }
    }

    #[test]
    fn the_line_under_a_place_is_found_as_the_view_puts_it() {
        // Two batches: a level line 0 to 100 at height 10 and one at height 30 (two segments); the view at 2 pixels a
        // unit, its origin 50 across and 200 down: a place across r, up u is at (50 + 2r, 200 − 2u).
        let list = LineList {
            key: "a".into(),
            items: vec![lines_of(7, &[strip_of([0.0, 10.0, 0.0], [100.0, 10.0, 0.0])], false), lines_of(9, &[strip_of([0.0, 30.0, 0.0], [40.0, 30.0, 0.0]), strip_of([40.0, 30.0, 0.0], [100.0, 30.0, 0.0])], false)],
        };
        // Three pixels under the first line, a quarter of the way along it.
        let hits = pick(&list, 2.0, 50.0, 200.0, None, 100.0, 183.0, 5.0, 8);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].item, hits[0].id, hits[0].segment, hits[0].inside), (0, 7, 0, false));
        assert!((hits[0].dist - 3.0).abs() < 1e-9 && (hits[0].along - 0.25).abs() < 1e-9, "{:?}", hits[0]);
        // Between the two (20 pixels from each): none within 5; both within 25, the nearer first.
        assert!(pick(&list, 2.0, 50.0, 200.0, None, 100.0, 160.0, 5.0, 8).is_empty());
        let hits = pick(&list, 2.0, 50.0, 200.0, None, 100.0, 159.0, 25.0, 8);
        assert_eq!(hits.iter().map(|h| (h.id, h.segment)).collect::<Vec<_>>(), vec![(9, 0), (7, 0)]);
        // The second batch's second segment (past 40 along: 130 across).
        let hits = pick(&list, 2.0, 50.0, 200.0, None, 200.0, 141.0, 5.0, 8);
        assert_eq!((hits[0].id, hits[0].segment), (9, 1));
        // Past a line's end: measured to the end (its `along` 1).
        let hits = pick(&list, 2.0, 50.0, 200.0, None, 253.0, 184.0, 5.0, 8);
        assert_eq!((hits[0].id, hits[0].along), (7, 1.0));
        assert!((hits[0].dist - 5.0).abs() < 1e-9);
        // `most`: the nearest only, of many.
        let many: Vec<[f32; 9]> = (0..500).map(|k| strip_of([0.0, k as f32 * 0.01, 0.0], [100.0, k as f32 * 0.01, 0.0])).collect();
        let list = LineList { key: "b".into(), items: vec![lines_of(1, &many, false)] };
        let hits = pick(&list, 1.0, 0.0, 0.0, None, 50.0, 0.0, 10.0, 3);
        assert_eq!(hits.iter().map(|h| h.segment).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert!(pick(&list, 1.0, 0.0, 0.0, None, 50.0, 0.0, 10.0, 0).is_empty());
    }

    #[test]
    fn lines_in_the_drawing_s_own_places_are_found_through_the_view_s_right_and_up() {
        // A line up the drawing's Z at x 10: seen from the south (right = +X, up = +Z) it's upright on screen.
        let list = LineList { key: "w".into(), items: vec![lines_of(3, &[strip_of([10.0, 0.0, 0.0], [10.0, 0.0, 50.0])], true)] };
        let south = LineWorld { right: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0], e: 100.0, f: 300.0 };
        // At 110 across, 300 − 25 down: on it, half way up.
        let hits = pick(&list, 1.0, 0.0, 0.0, Some(&south), 112.0, 275.0, 4.0, 4);
        assert_eq!(hits.len(), 1);
        assert!((hits[0].dist - 2.0).abs() < 1e-9 && (hits[0].along - 0.5).abs() < 1e-9);
        // With no right and up (a view at rest draws none of them): not found.
        assert!(pick(&list, 1.0, 0.0, 0.0, None, 112.0, 275.0, 4.0, 4).is_empty());
        // A turning view's letters aren't looked at.
        let letters = LineList { key: "l".into(), items: vec![LineItem { lettered: true, ..lines_of(4, &[strip_of([10.0, 0.0, 0.0], [10.0, 0.0, 50.0])], true) }] };
        assert!(pick(&letters, 1.0, 0.0, 0.0, Some(&south), 110.0, 275.0, 4.0, 4).is_empty());
    }

    #[test]
    fn a_place_inside_a_filled_shape_is_told_after_the_lines_near_it() {
        let outer: &[[f32; 2]] = &[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let hole: &[[f32; 2]] = &[[40.0, 40.0], [60.0, 40.0], [60.0, 60.0], [40.0, 60.0]];
        let back: Vec<[f32; 2]> = hole.iter().rev().copied().collect();
        let band: Vec<f32> = vec![0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 10.0, 10.0, 0.0, 200.0, 0.0, 0.0, 210.0, 0.0, 0.0, 210.0, 10.0, 0.0];
        let list = LineList {
            key: "s".into(),
            items: vec![
                shape_of(1, &[outer, hole], false),
                shape_of(2, &[outer, hole], true),
                shape_of(3, &[outer, &back], true),
                LineItem { fill: true, ..LineItem::solid(Arc::new(Strips { id: 4, floats: band }), [1.0; 4], false) },
                lines_of(5, &[strip_of([0.0, 20.0, 0.0], [100.0, 20.0, 0.0])], false),
            ],
        };
        // (Up 20 is 20 pixels up from the origin's height: y = −20.)
        let ids = |x: f64, y: f64| pick(&list, 1.0, 0.0, 0.0, None, x, y, 3.0, 8).iter().map(|h| (h.id, h.inside)).collect::<Vec<_>>();
        // In the square, off the hole and the line: every shape; none of the lines.
        assert_eq!(ids(20.0, -80.0), vec![(1, true), (2, true), (3, true)]);
        // In the hole: even-odd, a hole; nonzero, filled when it's wound as the outside is, a hole wound the other way.
        assert_eq!(ids(50.0, -50.0), vec![(2, true)]);
        // On the line, in the square: the line first, then the shapes.
        assert_eq!(ids(20.0, -21.0), vec![(5, false), (1, true), (2, true), (3, true)]);
        // Outside all: none. In the filled batch's second triangle: it, by the triangle.
        assert!(ids(150.0, -50.0).is_empty());
        let hits = pick(&list, 1.0, 0.0, 0.0, None, 208.0, -3.0, 3.0, 8);
        assert_eq!(hits.iter().map(|h| (h.id, h.segment, h.inside)).collect::<Vec<_>>(), vec![(4, 1, true)]);
    }

    #[test]
    fn a_strip_is_nine_floats_and_a_line_covers_its_middle() {
        let s = Strips { id: 1, floats: vec![0.0; 9 * 3 + 4] };
        assert_eq!(s.count(), 3);
        // The page's full line: 1.25 pixels wide. Its middle whole, a pixel off it a little, two off none.
        assert_eq!(covers(0.625, 0.0, 0.0), 1.0);
        assert!((covers(0.625, 1.0, 0.0) - 0.125).abs() < 1e-12);
        assert_eq!(covers(0.625, 2.0, 0.0), 0.0);
        // Round past a soft end.
        assert!((covers(0.625, 0.0, 1.0) - 0.125).abs() < 1e-12);
    }
}
