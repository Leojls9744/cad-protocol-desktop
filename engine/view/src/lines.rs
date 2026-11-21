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
//! Text's letters stay the page's for now.

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
#[derive(Clone)]
pub struct LineItem {
    pub strips: Arc<Strips>,
    pub rgba: [f32; 4],
    pub world: bool,
    pub dash: Vec<f32>,
    pub fill: bool,
}

impl LineItem {
    /// A solid batch of lines.
    pub fn solid(strips: Arc<Strips>, rgba: [f32; 4], world: bool) -> LineItem {
        LineItem { strips, rgba, world, dash: Vec::new(), fill: false }
    }
}

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
    // x: its ends are places in the drawing. y: how many dash lengths. z: the pattern's period.
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
    if (world) {
        ra = vec2<f32>(dot(a, v.right.xyz), dot(a, v.up.xyz));
        rb = vec2<f32>(dot(bb, v.right.xyz), dot(bb, v.up.xyz));
        at = v.wplace.xy;
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
    let reach = v.right.w + 1.0;
    var ext = caps.y * reach;
    if (corner.x < 0.5) {
        ext = caps.x * reach;
    }
    let t = corner.x * len + (corner.x * 2.0 - 1.0) * ext;
    let s = corner.y * reach;
    let p = pa + dir * t + nrm * s + v.size.zw;
    var out: VOut;
    out.pos = vec4<f32>(p.x / v.size.x * 2.0 - 1.0, 1.0 - p.y / v.size.y * 2.0, 0.0, 1.0);
    out.rel = vec3<f32>(s, t, len);
    out.caps = caps;
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
    var line = clamp(v.right.w + 0.5 - length(vec2<f32>(i.rel.x, past)), 0.0, 1.0);
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
}

/// The lines on the card: their strips (kept while the page keeps them), and what draws them.
pub(crate) struct LineCard {
    pipeline: wgpu::RenderPipeline,
    /// Filled batches' (triangles).
    fills: wgpu::RenderPipeline,
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
        LineCard { pipeline, fills, batch_layout, views, view_binds, listed: std::array::from_fn(|_| None), strips: HashMap::new(), step }
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
            let used = if item.fill { s.corners() as usize * 3 } else { s.count() as usize * STRIP_FLOATS };
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
        let same = self.listed[slot].as_ref().is_some_and(|l| l.key == lines.list.key && l.of == at(&lines.list));
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
                let u = BatchUniforms { color: item.rgba, kind: [if item.world { 1.0 } else { 0.0 }, n as f32, period_of(&item.dash), 0.0], dash };
                let at = k * self.step as usize;
                bytes[at..at + std::mem::size_of::<BatchUniforms>()].copy_from_slice(bytemuck::bytes_of(&u));
            }
            queue.write_buffer(&buffer, 0, &bytes);
            self.listed[slot] = Some(Listed { key: lines.list.key.clone(), of: at(&lines.list), buffer, bind, room });
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

    /// A view's lines drawn into its part of the window (the pass's scissor is the view's): each batch in turn, its segments as strips.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, slot: usize, lines: &Lines) {
        let Some(listed) = self.listed.get(slot).and_then(Option::as_ref) else { return };
        if listed.of != at(&lines.list) || listed.key != lines.list.key {
            return;
        }
        // The filled batches first, beneath the lines (a wide dashed polyline's dashes).
        if lines.list.items.iter().any(|i| i.fill) {
            pass.set_pipeline(&self.fills);
            pass.set_bind_group(0, &self.view_binds[slot], &[]);
            for (k, item) in lines.list.items.iter().enumerate() {
                if !item.fill {
                    continue;
                }
                let Some((s, buffer)) = self.strips.get(&item.strips.id) else { continue };
                if s.corners() == 0 || !Arc::ptr_eq(s, &item.strips) {
                    continue;
                }
                pass.set_bind_group(1, &listed.bind, &[(k as u64 * self.step) as u32]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..s.corners(), 0..1);
            }
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.view_binds[slot], &[]);
        for (k, item) in lines.list.items.iter().enumerate() {
            // In the drawing's own places: none without the view's right and up.
            if item.fill || (item.world && lines.world.is_none()) {
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
