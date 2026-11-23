//! The wgpu renderer of MurmelCraft. The window and `view.screenshot` use
//! the same [`MeshRenderer`]: it draws into its own texture (4x MSAA,
//! resolved to sRGB), which the window shows as an image and a screenshot
//! copies to a PNG. So every view the window can show is testable headless.

use std::fmt;
use std::path::Path;

use bytemuck::{Pod, Zeroable};
use murmel_engine::{Camera, Surface};

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
/// Largest screenshot side, to keep one call within memory.
pub const MAX_SIDE: u32 = 8192;

/// A render failure with its diagnostic code (`RENDER001` …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderError {
    pub code: &'static str,
    pub message: String,
}

impl RenderError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RenderError {}

/// The codes this crate reports, for `diag.codes`.
pub const CODES: &[(&str, &str, &str)] = &[
    (
        "RENDER001",
        "No GPU adapter was found (no driver, or no software renderer such as Mesa lavapipe).",
        "Install a Vulkan, Metal or DirectX 12 driver; on a headless Linux machine install mesa-vulkan-drivers.",
    ),
    (
        "RENDER002",
        "The GPU device could not be created or lost during rendering.",
        "Run again; set WGPU_BACKEND to choose another backend.",
    ),
    (
        "RENDER003",
        "The screenshot size is outside 1 to 8192 pixels per side.",
        "Pass width and height between 1 and 8192.",
    ),
    (
        "RENDER004",
        "The PNG could not be written.",
        "Check that the folder exists and is writable.",
    ),
];

/// A GPU device for offscreen work.
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: String,
}

impl Gpu {
    /// The first adapter wgpu finds (`WGPU_BACKEND` and
    /// `WGPU_ADAPTER_NAME` choose), software renderers included.
    pub fn headless() -> Result<Gpu, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|e| RenderError::new("RENDER001", format!("no GPU adapter: {e}")))?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("murmel-headless"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| RenderError::new("RENDER002", format!("no device: {e}")))?;
        Ok(Gpu {
            device,
            queue,
            adapter: format!("{} · {:?}", info.name, info.backend),
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    to_eye: [f32; 4],
    params: [f32; 4],
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
}

/// What to draw in one frame.
pub struct View<'a> {
    pub surface: Option<&'a Surface>,
    /// Changes whenever `surface` changes, so it is uploaded once.
    pub revision: u64,
    pub camera: &'a Camera,
    pub size: [u32; 2],
    /// Linear colours of the viewport gradient.
    pub bg_top: [f32; 3],
    pub bg_bottom: [f32; 3],
    pub edges: bool,
}

struct Targets {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    /// The same texels viewed as plain `Rgba8Unorm`: egui samples gamma
    /// encoded colours, so it must not decode the sRGB a second time.
    gamma_view: wgpu::TextureView,
}

struct Geometry {
    faces: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    edges: Option<wgpu::Buffer>,
    edge_count: u32,
}

pub struct MeshRenderer {
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    face_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    bg_pipeline: wgpu::RenderPipeline,
    geometry: Option<Geometry>,
    revision: Option<u64>,
    targets: Option<Targets>,
}

fn f32x3(v: [f32; 3]) -> [f32; 4] {
    [v[0], v[1], v[2], 0.0]
}

impl MeshRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("murmel-mesh"),
            source: wgpu::ShaderSource::Wgsl(include_str!("mesh.wgsl").into()),
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("murmel-globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("murmel-globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("murmel-globals"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("murmel"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        topology: wgpu::PrimitiveTopology,
                        depth: wgpu::DepthStencilState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil: Some(depth),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let face_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
        let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let face_pipeline = pipeline(
            "murmel-faces",
            "vs_face",
            "fs_face",
            &[Some(wgpu::VertexBufferLayout {
                array_stride: 36,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &face_attrs,
            })],
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                // Push faces back so their edges win the depth test.
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 1.5,
                    clamp: 0.0,
                },
            },
        );
        let line_pipeline = pipeline(
            "murmel-edges",
            "vs_line",
            "fs_line",
            &[Some(wgpu::VertexBufferLayout {
                array_stride: 24,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &line_attrs,
            })],
            wgpu::PrimitiveTopology::LineList,
            wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            },
        );
        let bg_pipeline = pipeline(
            "murmel-background",
            "vs_bg",
            "fs_bg",
            &[],
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            },
        );
        Self {
            globals,
            bind_group,
            face_pipeline,
            line_pipeline,
            bg_pipeline,
            geometry: None,
            revision: None,
            targets: None,
        }
    }

    fn upload(&mut self, device: &wgpu::Device, surface: Option<&Surface>) {
        use wgpu::util::DeviceExt;
        self.geometry = surface.filter(|s| !s.indices.is_empty()).map(|s| {
            let mut faces = Vec::with_capacity(s.positions.len() * 9);
            for i in 0..s.positions.len() {
                faces.extend_from_slice(&s.positions[i]);
                faces.extend_from_slice(&s.normals[i]);
                faces.extend_from_slice(&s.colors[i]);
            }
            let mut lines = Vec::with_capacity(s.edge_positions.len() * 6);
            for i in 0..s.edge_positions.len() {
                lines.extend_from_slice(&s.edge_positions[i]);
                lines.extend_from_slice(&s.edge_colors[i]);
            }
            let buf = |label: &str, data: &[u8], usage| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: data,
                    usage,
                })
            };
            Geometry {
                faces: buf(
                    "murmel-faces",
                    bytemuck::cast_slice(&faces),
                    wgpu::BufferUsages::VERTEX,
                ),
                indices: buf(
                    "murmel-indices",
                    bytemuck::cast_slice(&s.indices),
                    wgpu::BufferUsages::INDEX,
                ),
                index_count: s.indices.len() as u32,
                edges: (!lines.is_empty()).then(|| {
                    buf(
                        "murmel-edges",
                        bytemuck::cast_slice(&lines),
                        wgpu::BufferUsages::VERTEX,
                    )
                }),
                edge_count: s.edge_positions.len() as u32,
            }
        });
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let tex = |label: &str, format, samples, usage, view_formats: &[wgpu::TextureFormat]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let msaa = tex(
            "murmel-msaa",
            FORMAT,
            SAMPLES,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            &[],
        );
        let depth = tex(
            "murmel-depth",
            DEPTH,
            SAMPLES,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            &[],
        );
        let resolve = tex(
            "murmel-view",
            FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            &[wgpu::TextureFormat::Rgba8Unorm],
        );
        self.targets = Some(Targets {
            size,
            msaa: msaa.create_view(&Default::default()),
            depth: depth.create_view(&Default::default()),
            resolve_view: resolve.create_view(&Default::default()),
            gamma_view: resolve.create_view(&wgpu::TextureViewDescriptor {
                format: Some(wgpu::TextureFormat::Rgba8Unorm),
                ..Default::default()
            }),
            resolve,
        });
    }

    /// Draw `view` into the renderer's texture and submit the work.
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, view: &View<'_>) {
        let size = [view.size[0].max(1), view.size[1].max(1)];
        if self.revision != Some(view.revision) {
            self.upload(device, view.surface);
            self.revision = Some(view.revision);
        }
        self.ensure_targets(device, size);
        let (origin, radius) = view
            .surface
            .map_or(([0.0; 3], 1.0), |s| (s.origin, s.radius));
        let frame = view
            .camera
            .frame(origin, radius, size[0] as f64 / size[1] as f64);
        let g = Globals {
            view_proj: frame.view_proj.to_cols_array_2d(),
            eye: f32x3(frame.eye),
            to_eye: f32x3(frame.light),
            params: [if frame.orthographic { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            bg_top: f32x3(view.bg_top),
            bg_bottom: f32x3(view.bg_bottom),
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        let Some(t) = &self.targets else { return };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("murmel-frame"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("murmel-view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.msaa,
                    depth_slice: None,
                    resolve_target: Some(&t.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(&self.bg_pipeline);
            pass.draw(0..3, 0..1);
            if let Some(geo) = &self.geometry {
                pass.set_pipeline(&self.face_pipeline);
                pass.set_vertex_buffer(0, geo.faces.slice(..));
                pass.set_index_buffer(geo.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..geo.index_count, 0, 0..1);
                if view.edges
                    && let Some(edges) = &geo.edges
                {
                    pass.set_pipeline(&self.line_pipeline);
                    pass.set_vertex_buffer(0, edges.slice(..));
                    pass.draw(0..geo.edge_count, 0..1);
                }
            }
        }
        queue.submit([encoder.finish()]);
    }

    /// The rendered image (sRGB), valid after [`MeshRenderer::render`].
    pub fn texture(&self) -> Option<&wgpu::Texture> {
        self.targets.as_ref().map(|t| &t.resolve)
    }

    pub fn texture_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.resolve_view)
    }

    /// The image as gamma-encoded `Rgba8Unorm`, the way egui samples its
    /// own textures; use this view to show the image in egui.
    pub fn egui_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.gamma_view)
    }

    /// Read the rendered image back as tightly packed RGBA8 rows.
    pub fn read_rgba(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<u8>, RenderError> {
        let t = self
            .targets
            .as_ref()
            .ok_or_else(|| RenderError::new("RENDER002", "nothing rendered yet"))?;
        let [w, h] = t.size;
        let row = 4 * w;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("murmel-readback"),
            size: padded as u64 * h as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &t.resolve,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::new("RENDER002", format!("poll: {e}")))?;
        rx.recv()
            .map_err(|e| RenderError::new("RENDER002", format!("map: {e}")))?
            .map_err(|e| RenderError::new("RENDER002", format!("map: {e}")))?;
        let data = slice
            .get_mapped_range()
            .map_err(|e| RenderError::new("RENDER002", format!("map: {e}")))?;
        let mut out = Vec::with_capacity((row * h) as usize);
        for y in 0..h as usize {
            let start = y * padded as usize;
            out.extend_from_slice(&data[start..start + row as usize]);
        }
        drop(data);
        buffer.unmap();
        Ok(out)
    }
}

/// An offscreen image of the session's view.
pub struct Shot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub adapter: String,
}

/// Render `session` at `width` × `height` on a headless GPU.
pub fn render_session(
    session: &murmel_engine::Session,
    width: u32,
    height: u32,
) -> Result<Shot, RenderError> {
    if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
        return Err(RenderError::new(
            "RENDER003",
            format!("{width} x {height} is outside 1..{MAX_SIDE}"),
        ));
    }
    let gpu = Gpu::headless()?;
    let mut r = MeshRenderer::new(&gpu.device);
    let dark = session.theme.is_dark(None);
    let p = session.tokens.palette(dark);
    r.render(
        &gpu.device,
        &gpu.queue,
        &View {
            surface: session.mesh.as_ref().map(|m| &m.surface),
            revision: session.surface_revision,
            camera: &session.camera,
            size: [width, height],
            bg_top: p.viewport_top.linear(),
            bg_bottom: p.viewport_bottom.linear(),
            edges: true,
        },
    );
    let rgba = r.read_rgba(&gpu.device, &gpu.queue)?;
    Ok(Shot {
        width,
        height,
        rgba,
        adapter: gpu.adapter,
    })
}

/// Encode RGBA8 rows as a PNG file.
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), RenderError> {
    let file = std::fs::File::create(path)
        .map_err(|e| RenderError::new("RENDER004", format!("{}: {e}", path.display())))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut w = enc
        .write_header()
        .map_err(|e| RenderError::new("RENDER004", format!("{}: {e}", path.display())))?;
    w.write_image_data(rgba)
        .map_err(|e| RenderError::new("RENDER004", format!("{}: {e}", path.display())))?;
    w.finish()
        .map_err(|e| RenderError::new("RENDER004", format!("{}: {e}", path.display())))
}

/// Decode a PNG into RGBA8 (for render tests).
pub fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| "PNG too large".to_string())?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(format!("{}: not 8-bit RGBA", path.display()));
    }
    Ok((info.width, info.height, buf))
}

/// The share of pixels whose largest channel difference exceeds `tol`.
pub fn diff_share(a: &[u8], b: &[u8], tol: u8) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let bad = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > tol))
        .count();
    bad as f64 / (a.len() / 4) as f64
}
