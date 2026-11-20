//! The card: a scan's points kept on it, and a frame drawn from them.
//!
//! Each point is a small square facing the eye (two triangles made on the
//! card from the one point kept), so its size is in pixels whatever the
//! zoom. Nearer points cover further ones (a depth buffer). Drawn into a
//! window's surface, or into a picture of the engine's own (the tests', and
//! the build's check where there's no window).

use crate::camera::{columns, Camera};
use crate::scene::{PointVertex, Points};

/// What the page behind shows where nothing's drawn: the drawing's own background is the page's to give.
pub const CLEAR: [f64; 4] = [0.043, 0.063, 0.125, 1.0];

const SHADER: &str = r#"
struct U {
  m: mat4x4<f32>,
  size: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;

struct VOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) color: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @location(0) p: vec3<f32>, @location(1) c: vec4<f32>) -> VOut {
  var corners = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
    vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
  );
  let q = corners[vi];
  let centre = u.m * vec4<f32>(p, 1.0);
  var o: VOut;
  o.pos = vec4<f32>(centre.xy + q * u.size.xy, centre.z, 1.0);
  o.color = c;
  return o;
}

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
  return vec4<f32>(i.color.rgb, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    m: [[f32; 4]; 4],
    size: [f32; 4],
}

/// A scan's points on the card.
pub struct Cloud {
    buffer: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
    pub count: u32,
    pub centre: [f64; 3],
    pub radius: f64,
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
    depth: Option<(wgpu::TextureView, [u32; 2])>,
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
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cadp points"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
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
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 12, shader_location: 1 },
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
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Ok(Gpu { device, queue, format, adapter: name, pipeline, layout, depth: None })
    }

    /// A scan's points sent to the card.
    pub fn upload(&self, points: &Points) -> Cloud {
        let bytes: &[u8] = bytemuck::cast_slice(&points.data);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cadp points"),
            size: (bytes.len().max(16)) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // A part at a time, so no one write is past what the card's queue takes.
        const PART: usize = 64 << 20;
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
        Cloud { buffer, uniforms, bind, count: points.data.len() as u32, centre: points.centre, radius: points.radius }
    }

    fn depth_for(&mut self, size: [u32; 2]) -> wgpu::TextureView {
        if let Some((view, had)) = &self.depth {
            if *had == size {
                return view.clone();
            }
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cadp depth"),
            size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.depth = Some((view.clone(), size));
        view
    }

    /// A frame: each scan through the view, into `target` (`size` pixels),
    /// its points `px` pixels across. With no view, the background alone.
    pub fn draw(
        &mut self,
        target: &wgpu::TextureView,
        size: [u32; 2],
        camera: Option<&Camera>,
        clouds: &[(&Cloud, crate::camera::Affine)],
        px: f64,
        clear: [f64; 4],
    ) {
        let depth = self.depth_for(size);
        // How far the scans reach from the target along the line of sight: what the depth runs over.
        let mut reach = 1.0f64;
        if let Some(cam) = camera {
            for (cloud, affine) in clouds {
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
            for (cloud, affine) in clouds {
                let m = cam.matrix(cloud.centre, affine, size, reach * 1.05);
                let u = Uniforms {
                    m: columns(&m),
                    size: [(px / size[0].max(1) as f64) as f32, (px / size[1].max(1) as f64) as f32, 0.0, 0.0],
                };
                self.queue.write_buffer(&cloud.uniforms, 0, bytemuck::bytes_of(&u));
            }
        }
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cadp frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cadp frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0], g: clear[1], b: clear[2], a: clear[3] }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some(cam) = camera {
                if let Some([x, y, w, h]) = cam.scissor(size) {
                    pass.set_scissor_rect(x, y, w, h);
                    pass.set_pipeline(&self.pipeline);
                    for (cloud, _) in clouds {
                        if cloud.count == 0 {
                            continue;
                        }
                        pass.set_bind_group(0, &cloud.bind, &[]);
                        pass.set_vertex_buffer(0, cloud.buffer.slice(..));
                        pass.draw(0..6, 0..cloud.count);
                    }
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
    }

    /// A frame drawn into a picture of the engine's own, and read back: its
    /// pixels, red green blue and how solid, row by row from the top.
    pub fn picture(
        &mut self,
        size: [u32; 2],
        camera: Option<&Camera>,
        clouds: &[(&Cloud, crate::camera::Affine)],
        px: f64,
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
        self.draw(&view, size, camera, clouds, px, clear);
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
