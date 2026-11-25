//! The wgpu renderer of MurmelCraft. The window and `view.screenshot` use
//! the same [`MeshRenderer`]: it draws into its own texture (4x MSAA,
//! resolved to sRGB), which the window shows as an image and a screenshot
//! copies to a PNG. So every view the window can show is testable headless.
//! Exported pictures (`export.image`, `export.movie`) add a transparent
//! background, lines wider than one pixel and the overlays of [`overlay`]
//! (legend and axis triad), drawn on the CPU.

pub mod overlay;

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
    (
        "RENDER005",
        "The font of the overlays (Inter) could not be read.",
        "Rebuild MurmelCraft; assets/fonts/Inter-Regular.ttf is part of the build.",
    ),
    (
        "RENDER006",
        "The GPU refused a step of drawing the view (a wgpu validation error, or out of GPU memory); the window goes on, but the view may lack what that step drew.",
        "Report it with the message; hiding parts of the mesh or a region of it draws less.",
    ),
];

/// The errors of a device that no error scope takes, kept for the window
/// to show (RENDER006) instead of wgpu's default, which ends the program
/// and loses the session. The first since the last [`GpuErrors::take`] is
/// kept: the errors after it mostly follow from it.
#[derive(Clone, Default)]
pub struct GpuErrors(std::sync::Arc<std::sync::Mutex<Option<String>>>);

impl GpuErrors {
    /// Catch the errors of `device` from now on.
    pub fn catch(device: &wgpu::Device) -> GpuErrors {
        let errors = GpuErrors::default();
        let keep = errors.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |e: wgpu::Error| keep.keep(&e)));
        errors
    }

    fn keep(&self, e: &wgpu::Error) {
        let mut first = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if first.is_none() {
            *first = Some(e.to_string());
        }
    }

    /// The first error caught since the last call, if any.
    pub fn take(&self) -> Option<RenderError> {
        let mut first = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        first.take().map(|m| RenderError::new("RENDER006", m))
    }
}

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

    /// The process's one offscreen device, made on first use and kept to
    /// the end. Offscreen renders (screenshots, render tests) share it
    /// instead of each creating and dropping its own instance: Mesa 26
    /// lavapipe crashed (SIGSEGV) when render tests created and destroyed
    /// devices on several threads at once (docs/pitfalls.md).
    pub fn shared() -> Result<&'static std::sync::Mutex<Gpu>, RenderError> {
        static GPU: std::sync::OnceLock<Result<std::sync::Mutex<Gpu>, RenderError>> =
            std::sync::OnceLock::new();
        GPU.get_or_init(|| Gpu::headless().map(std::sync::Mutex::new))
            .as_ref()
            .map_err(Clone::clone)
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
    field: [f32; 4],
    field_mode: [f32; 4],
    contour: [f32; 4],
    selection: [f32; 4],
    clip_planes: [[f32; 4]; MAX_PREVIEW_PLANES],
    clip_count: [f32; 4],
}

/// Clip planes a frame can cut with on the GPU (`View::clip_planes`; the
/// display has at most eight).
pub const MAX_PREVIEW_PLANES: usize = 8;

/// Width of the contour lines in pixels.
const LINE_PX: f32 = 1.5;

/// What to draw in one frame.
pub struct View<'a> {
    /// Width of edge and outline lines in pixels: 1 in the window, the
    /// export scale in exported pictures (lines wider than one pixel are
    /// drawn as screen-aligned quads).
    pub line_width: f32,
    /// Leave the background out (alpha 0) instead of the gradient.
    pub transparent: bool,
    pub surface: Option<&'a Surface>,
    /// Changes whenever `surface` changes, so it is uploaded once.
    pub revision: u64,
    pub camera: &'a Camera,
    pub size: [u32; 2],
    /// Linear colours of the viewport gradient.
    pub bg_top: [f32; 3],
    pub bg_bottom: [f32; 3],
    /// Draw the shaded faces (render modes 1 and 2).
    pub faces: bool,
    /// Draw the edge lines (render modes 1, 3 and 4).
    pub edges: bool,
    /// Colour mode field: how the faces with values are coloured.
    pub field: Option<&'a murmel_engine::FieldPaint>,
    /// Physical pixels per logical pixel, so contour lines keep their
    /// width on a HiDPI screen.
    pub scale: f32,
    /// Planes that cut the surface on the GPU (`[n, d]` relative to the
    /// surface's origin; a point `p` is cut away where `n · p + d > 0`):
    /// the window's clip preview while the exact cut is built
    /// ([`murmel_engine::Session::drawn`]); empty otherwise.
    pub clip_planes: &'a [[f32; 4]],
    /// The thin layer of a clip drag ([`murmel_engine::Drawn::layer`])
    /// and its revision: drawn over `surface`, not cut by `clip_planes`.
    pub layer: Option<(&'a Surface, u64)>,
    /// The slices dragged in the window, drawn from the field's grid
    /// while their exact faces are built ([`murmel_engine::Session::slice_drawn`]).
    pub slice: Option<&'a murmel_engine::slice_preview::SliceDraw>,
    /// The iso-surface whose value changed in the window, ray cast
    /// through its field's grid while its exact faces are built
    /// ([`murmel_engine::Session::iso_drawn`]).
    pub iso: Option<&'a murmel_engine::slice_preview::IsoDraw>,
}

/// The iso preview's uniform (`IsoGrid` in the shader).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct IsoUniform {
    lo: [f32; 4],
    size: [f32; 4],
    dims: [f32; 4],
    color: [f32; 4],
    view: [f32; 4],
    clips: [[f32; 4]; MAX_PREVIEW_PLANES],
}

/// The slice preview's uniform (`SliceGrid` in the shader).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SliceUniform {
    lo: [f32; 4],
    side: [f32; 4],
    dims: [f32; 4],
    clips: [[f32; 4]; MAX_PREVIEW_PLANES],
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

/// A surface's part as GPU buffers, each at most [`MeshRenderer`]'s
/// piece size: one piece for most meshes, several for a surface larger
/// than a GPU buffer may be (the boundary of a 100 M cell overview takes
/// 1.6 GB of face vertices, a device gives 256 MiB per buffer).
struct Geometry {
    faces: Vec<FacePiece>,
    index_count: u32,
    edges: Vec<LinePiece>,
    edge_count: u32,
    /// The outline of the picked cells and faces, drawn over everything.
    outline: Vec<LinePiece>,
    outline_count: u32,
    /// Streamlines drawn as lines: in every render mode, behind faces.
    streams: Vec<LinePiece>,
    stream_count: u32,
}

/// Triangles of a part and the vertices they use: the indices `at` of
/// the part's index list, numbered from the first vertex of the piece.
struct FacePiece {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    at: std::ops::Range<u32>,
}

/// The vertices `at` of a part's line list (an even count, so no segment
/// is split).
struct LinePiece {
    vertices: wgpu::Buffer,
    at: std::ops::Range<u32>,
}

/// The pieces holding the elements `r`, each with the elements within it.
fn within<'a, P>(
    pieces: &'a [P],
    at: impl Fn(&P) -> std::ops::Range<u32> + 'a,
    r: std::ops::Range<u32>,
) -> impl Iterator<Item = (&'a P, std::ops::Range<u32>)> + 'a {
    pieces.iter().filter_map(move |p| {
        let a = at(p);
        let (start, end) = (r.start.max(a.start), r.end.min(a.end));
        (start < end).then(|| (p, start - a.start..end - a.start))
    })
}

pub struct MeshRenderer {
    globals: wgpu::Buffer,
    /// The colour table of the field's colour map, and which map and
    /// direction it holds.
    lut: wgpu::Texture,
    lut_key: Option<(&'static str, bool)>,
    bind_group: wgpu::BindGroup,
    face_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    /// Lines wider than one pixel: each segment an instance of two
    /// triangles, widened on screen ([`View::line_width`]).
    wide_pipeline: wgpu::RenderPipeline,
    wide_outline_pipeline: wgpu::RenderPipeline,
    bg_pipeline: wgpu::RenderPipeline,
    /// The planes of slices being dragged, coloured from the field's grid.
    slice_pipeline: wgpu::RenderPipeline,
    /// What the bind group is made from: its layout, the colour table's
    /// view and sampler, the slice preview's uniform and grid (a 1 × 1 × 1
    /// texture until a grid is uploaded) and that grid's id.
    layout: wgpu::BindGroupLayout,
    lut_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    slice_uniform: wgpu::Buffer,
    grid: wgpu::Texture,
    grid_id: Option<u64>,
    /// The iso preview: its pipeline (the grid's box, its back faces),
    /// uniform, and grid (`Rg16Float`, mixed by the sampler) with its id.
    iso_pipeline: wgpu::RenderPipeline,
    iso_uniform: wgpu::Buffer,
    iso_grid: wgpu::Texture,
    iso_id: Option<u64>,
    /// The mesh's own faces and the extras (slices, iso-surfaces, arrows
    /// and the like), uploaded apart so a change of the extras leaves the
    /// mesh's buffers as they are.
    geometry: [Option<Geometry>; 2],
    /// The build key of the mesh's faces uploaded.
    mesh_key: Option<u64>,
    revision: Option<u64>,
    /// The thin layer of a clip drag ([`View::layer`]) and its revision.
    layer: Option<Geometry>,
    layer_revision: Option<u64>,
    targets: Option<Targets>,
    /// The most bytes one buffer of the surface takes: [`PIECE_BYTES`],
    /// or less where the device allows less.
    piece: u64,
}

/// Which part of a surface a [`Geometry`] holds.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    /// The mesh's own faces, edges and the outline of what is picked.
    Mesh,
    /// The slices, iso-surfaces, arrows, streamlines, parcels and block
    /// boxes ([`murmel_engine::surface::ExtrasAt`]).
    Extras,
}

/// The most bytes one buffer of a [`Geometry`] takes: wgpu's default
/// limit, which every device gives without asking (the window's device
/// asks for no more).
const PIECE_BYTES: u64 = 1 << 28;

/// The buffers of part `part` of `s`, none over `piece` bytes; `None`
/// when it has nothing to draw.
fn geometry(device: &wgpu::Device, s: &Surface, part: Part, piece: u64) -> Option<Geometry> {
    let at = s.extras_at;
    // A surface built before the extras were marked holds them all in
    // the mesh's part.
    let (v0, v1, i0, i1, e0, e1) = match part {
        Part::Mesh => (0, at.vertices, 0, at.indices, 0, at.edges),
        Part::Extras => (
            at.vertices,
            s.positions.len(),
            at.indices,
            s.indices.len(),
            at.edges,
            s.edge_positions.len(),
        ),
    };
    let (v1, i1, e1) = (
        v1.min(s.positions.len()),
        i1.min(s.indices.len()),
        e1.min(s.edge_positions.len()),
    );
    let (v0, i0, e0) = (v0.min(v1), i0.min(i1), e0.min(e1));
    let faces = face_pieces(device, s, v0..v1, i0..i1, piece);
    let edges = line_pieces(
        device,
        "murmel-edges",
        &s.edge_positions[e0..e1],
        &s.edge_colors[e0.min(s.edge_colors.len())..e1.min(s.edge_colors.len())],
        None,
        piece,
    );
    let (outline, streams) = match part {
        Part::Mesh => (
            line_pieces(
                device,
                "murmel-outline",
                &s.outline,
                &[],
                Some(s.outline_color),
                piece,
            ),
            Vec::new(),
        ),
        Part::Extras => (
            Vec::new(),
            line_pieces(
                device,
                "murmel-streams",
                &s.stream_positions,
                &s.stream_colors,
                None,
                piece,
            ),
        ),
    };
    // What the pieces hold, counted as the part's elements (a piece left
    // out leaves a gap).
    let count = |at: Option<&std::ops::Range<u32>>| at.map_or(0, |r| r.end);
    let g = Geometry {
        index_count: count(faces.last().map(|p| &p.at)),
        edge_count: count(edges.last().map(|p| &p.at)),
        outline_count: count(outline.last().map(|p| &p.at)),
        stream_count: count(streams.last().map(|p| &p.at)),
        faces,
        edges,
        outline,
        streams,
    };
    (g.index_count > 0 || g.edge_count > 0 || g.outline_count > 0 || g.stream_count > 0)
        .then_some(g)
}

/// A vertex buffer holding `data`, or `None` beyond `piece` bytes (a
/// piece of one polygon wider than that: never made by the surface
/// builders, and left out rather than failing the device).
fn vertex_buffer(
    device: &wgpu::Device,
    label: &str,
    data: &[u8],
    usage: wgpu::BufferUsages,
    piece: u64,
) -> Option<wgpu::Buffer> {
    use wgpu::util::DeviceExt;
    (!data.is_empty() && data.len() as u64 <= piece).then(|| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: data,
            usage,
        })
    })
}

/// The triangles `tris` (positions in `s.indices`) of the vertices
/// `verts` as pieces of at most `piece` bytes each. A piece takes the
/// triangles in order while the vertices they span and their indices
/// fit; the builders add each polygon's vertices and triangles together,
/// so the pieces hold every vertex once.
fn face_pieces(
    device: &wgpu::Device,
    s: &Surface,
    verts: std::ops::Range<usize>,
    tris: std::ops::Range<usize>,
    piece: u64,
) -> Vec<FacePiece> {
    let indices = &s.indices[tris.start..tris.start + (tris.len() / 3) * 3];
    let (lo_ok, hi_ok) = (verts.start as u32, verts.end as u32);
    let mut pieces = Vec::new();
    let mut t = 0;
    while t < indices.len() {
        let start = t;
        let (mut lo, mut hi) = (u32::MAX, 0u32);
        while t < indices.len() {
            let tri = &indices[t..t + 3];
            let (a, b) = (
                lo.min(tri[0].min(tri[1]).min(tri[2])),
                hi.max(tri[0].max(tri[1]).max(tri[2])),
            );
            let span = u64::from(b - a.min(b)) + 1;
            let fits = span * FACE_STRIDE <= piece && (t + 3 - start) as u64 * 4 <= piece;
            if t > start && !fits {
                break;
            }
            (lo, hi) = (a, b);
            t += 3;
        }
        // Vertices outside the part's (a broken surface) are left out.
        let (lo, hi) = (lo.max(lo_ok), hi.min(hi_ok.saturating_sub(1)));
        let mut data: Vec<f32> = Vec::with_capacity((hi + 1).saturating_sub(lo) as usize * 13);
        for i in lo as usize..=hi as usize {
            if i >= hi_ok as usize {
                break;
            }
            data.extend_from_slice(&s.positions[i]);
            data.extend_from_slice(s.normals.get(i).unwrap_or(&[0.0; 3]));
            data.extend_from_slice(s.colors.get(i).unwrap_or(&[0.0; 3]));
            data.extend_from_slice(s.values.get(i).unwrap_or(&[0.0; 4]));
        }
        let local: Vec<u32> = indices[start..t]
            .iter()
            .map(|&k| k.saturating_sub(lo))
            .collect();
        let at = (start as u32)..(t as u32);
        let v = vertex_buffer(
            device,
            "murmel-faces",
            bytemuck::cast_slice(&data),
            wgpu::BufferUsages::VERTEX,
            piece,
        );
        let i = vertex_buffer(
            device,
            "murmel-indices",
            bytemuck::cast_slice(&local),
            wgpu::BufferUsages::INDEX,
            piece,
        );
        if let (Some(vertices), Some(indices)) = (v, i) {
            pieces.push(FacePiece {
                vertices,
                indices,
                at,
            });
        }
    }
    pieces
}

/// The line list `points` (each with its colour from `colors`, or
/// `color`) as pieces of at most `piece` bytes, an even number of
/// vertices each.
fn line_pieces(
    device: &wgpu::Device,
    label: &str,
    points: &[[f32; 3]],
    colors: &[[f32; 3]],
    color: Option<[f32; 3]>,
    piece: u64,
) -> Vec<LinePiece> {
    let per = ((piece / LINE_STRIDE) as usize / 2 * 2).max(2);
    let mut pieces = Vec::new();
    for (k, chunk) in points.chunks(per).enumerate() {
        let first = k * per;
        let mut data: Vec<f32> = Vec::with_capacity(chunk.len() * 6);
        for (j, p) in chunk.iter().enumerate() {
            data.extend_from_slice(p);
            let c = color.or_else(|| colors.get(first + j).copied());
            data.extend_from_slice(&c.unwrap_or([0.0; 3]));
        }
        if let Some(vertices) = vertex_buffer(
            device,
            label,
            bytemuck::cast_slice(&data),
            wgpu::BufferUsages::VERTEX,
            piece,
        ) {
            pieces.push(LinePiece {
                vertices,
                at: first as u32..(first + chunk.len()) as u32,
            });
        }
    }
    pieces
}

/// Bytes per face vertex: position, normal, colour, field values.
const FACE_STRIDE: u64 = 4 * (3 + 3 + 3 + 4);
/// Bytes per line vertex: position, colour.
const LINE_STRIDE: u64 = 4 * (3 + 3);

fn f32x3(v: [f32; 3]) -> [f32; 4] {
    [v[0], v[1], v[2], 0.0]
}

/// A grid texture of `dims` cells, two values each in `format`.
fn grid_texture(
    device: &wgpu::Device,
    dims: [u32; 3],
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("murmel-preview-grid"),
        size: wgpu::Extent3d {
            width: dims[0].max(1),
            height: dims[1].max(1),
            depth_or_array_layers: dims[2].max(1),
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    lut_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    (grid, slice_uniform): (&wgpu::Texture, &wgpu::Buffer),
    (iso_grid, iso_uniform): (&wgpu::Texture, &wgpu::Buffer),
) -> wgpu::BindGroup {
    let grid_view = grid.create_view(&Default::default());
    let iso_view = iso_grid.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("murmel-globals"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(lut_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&grid_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: slice_uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&iso_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: iso_uniform.as_entire_binding(),
            },
        ],
    })
}

/// Upload `texels` (`bytes` bytes each) into a new grid texture of `dims`
/// cells in `format`; `None` when they do not fill it.
fn upload_grid<T: Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    dims: [u32; 3],
    format: wgpu::TextureFormat,
    bytes: u32,
    texels: &[T],
) -> Option<wgpu::Texture> {
    let size = wgpu::Extent3d {
        width: dims[0].max(1),
        height: dims[1].max(1),
        depth_or_array_layers: dims[2].max(1),
    };
    let n = (size.width * size.height * size.depth_or_array_layers) as usize;
    if texels.len() != n {
        return None;
    }
    let texture = grid_texture(device, dims, format);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(texels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.width * bytes),
            rows_per_image: Some(size.height),
        },
        size,
    );
    Some(texture)
}

/// The 36 corners of the twelve triangles of the box from `lo` to `hi`.
fn box_vertices(lo: [f32; 3], hi: [f32; 3]) -> Vec<f32> {
    let c = |i: usize| [0, 1, 2].map(|k| if i >> k & 1 == 1 { hi[k] } else { lo[k] });
    // Each face as two triangles, wound outward.
    const FACES: [[usize; 4]; 6] = [
        [0, 2, 6, 4],
        [1, 5, 7, 3],
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 1, 3, 2],
        [4, 6, 7, 5],
    ];
    let mut out = Vec::with_capacity(36 * 3);
    for f in FACES {
        for i in [f[0], f[1], f[2], f[0], f[2], f[3]] {
            out.extend_from_slice(&c(i));
        }
    }
    out
}

/// The triangles of the slice preview's planes (each convex polygon a
/// fan), as face vertices: the slice's colour, and the selection mix.
fn slice_vertices(d: &murmel_engine::slice_preview::SliceDraw) -> Vec<f32> {
    let mut out = Vec::new();
    for (poly, n) in &d.polygons {
        for i in 1..poly.len().saturating_sub(1) {
            for p in [poly[0], poly[i], poly[i + 1]] {
                out.extend_from_slice(&p);
                out.extend_from_slice(n);
                out.extend_from_slice(&d.color);
                out.extend_from_slice(&[0.0, 0.0, d.selection_mix, 0.0]);
            }
        }
    }
    out
}

impl MeshRenderer {
    /// Put the previews' grids on the GPU (each once) and their uniforms;
    /// the vertices of the slice planes and of the iso grid's box, if
    /// any. `to_eye` is the unit vector from the target to the eye.
    fn prepare_previews(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &View<'_>,
        to_eye: [f32; 3],
    ) -> [Option<(wgpu::Buffer, u32)>; 2] {
        use wgpu::util::DeviceExt;
        let (slice, iso) = (view.slice, view.iso);
        // The field shown: what slice planes, and iso-surfaces coloured by
        // it, read.
        let grid = slice
            .and_then(|d| d.grid.as_deref())
            .or_else(|| iso.and_then(|d| d.field.as_deref()));
        let mut changed = false;
        if let Some(g) = grid
            && self.grid_id != Some(g.id)
            && let Some(t) = upload_grid(
                device,
                queue,
                g.dims,
                wgpu::TextureFormat::Rg32Float,
                8,
                &g.texels,
            )
        {
            self.grid = t;
            self.grid_id = Some(g.id);
            changed = true;
        }
        if let Some(d) = iso
            && self.iso_id != Some(d.texels.id)
            && let Some(t) = upload_grid(
                device,
                queue,
                d.grid.dims,
                wgpu::TextureFormat::Rg16Float,
                4,
                &d.texels.texels,
            )
        {
            self.iso_grid = t;
            self.iso_id = Some(d.texels.id);
            changed = true;
        }
        if changed {
            self.bind_group = bind_group(
                device,
                &self.layout,
                &self.globals,
                &self.lut_view,
                &self.sampler,
                (&self.grid, &self.slice_uniform),
                (&self.iso_grid, &self.iso_uniform),
            );
        }
        let has_grid = grid.is_some_and(|g| self.grid_id == Some(g.id));
        let clips = |c: &[[f32; 4]]| -> [[f32; 4]; MAX_PREVIEW_PLANES] {
            std::array::from_fn(|i| c.get(i).copied().unwrap_or([0.0; 4]))
        };
        let slice_clips = slice.map_or(&[][..], |d| d.clips.as_slice());
        let u = SliceUniform {
            lo: grid.map_or([0.0; 4], |g| {
                let planes = slice.is_some_and(|d| d.grid.is_some());
                [
                    g.lo[0],
                    g.lo[1],
                    g.lo[2],
                    if has_grid && planes { 1.0 } else { 0.0 },
                ]
            }),
            side: grid.map_or([1.0; 4], |g| [g.side[0], g.side[1], g.side[2], 0.0]),
            dims: {
                let dims = grid.map_or([1; 3], |g| g.dims);
                [
                    dims[0] as f32,
                    dims[1] as f32,
                    dims[2] as f32,
                    slice_clips.len().min(MAX_PREVIEW_PLANES) as f32,
                ]
            },
            clips: clips(slice_clips),
        };
        queue.write_buffer(&self.slice_uniform, 0, bytemuck::bytes_of(&u));
        let buffer = |label: &str, verts: &[f32], stride: usize| {
            (!verts.is_empty()).then(|| {
                let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::cast_slice(verts),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                (buf, (verts.len() / stride) as u32)
            })
        };
        let planes = slice.and_then(|d| buffer("murmel-slice-planes", &slice_vertices(d), 13));
        let iso_box = iso
            .filter(|d| self.iso_id == Some(d.texels.id))
            .and_then(|d| {
                let g = &d.grid;
                let size: [f32; 3] = std::array::from_fn(|k| g.side[k] * g.dims[k] as f32);
                let u = IsoUniform {
                    lo: [g.lo[0], g.lo[1], g.lo[2], 0.0],
                    size: [
                        size[0],
                        size[1],
                        size[2],
                        if d.field.is_some() && has_grid {
                            1.0
                        } else {
                            0.0
                        },
                    ],
                    dims: [
                        g.dims[0] as f32,
                        g.dims[1] as f32,
                        g.dims[2] as f32,
                        d.clips.len().min(MAX_PREVIEW_PLANES) as f32,
                    ],
                    color: [d.color[0], d.color[1], d.color[2], d.selection_mix],
                    view: f32x3(to_eye),
                    clips: clips(&d.clips),
                };
                queue.write_buffer(&self.iso_uniform, 0, bytemuck::bytes_of(&u));
                let hi: [f32; 3] = std::array::from_fn(|k| g.lo[k] + size[k]);
                buffer("murmel-iso-box", &box_vertices(g.lo, hi), 3)
            });
        [planes, iso_box]
    }

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
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        // The colour table: LUT_SIZE sRGB texels in one row, mixed
        // linearly between texels.
        let lut = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("murmel-colormap"),
            size: wgpu::Extent3d {
                width: murmel_engine::colormap::LUT_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("murmel-colormap"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let lut_view = lut.create_view(&Default::default());
        let slice_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("murmel-slice-grid"),
            size: std::mem::size_of::<SliceUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grid = grid_texture(device, [1, 1, 1], wgpu::TextureFormat::Rg32Float);
        let iso_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("murmel-iso-grid"),
            size: std::mem::size_of::<IsoUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let iso_grid = grid_texture(device, [1, 1, 1], wgpu::TextureFormat::Rg16Float);
        let bind_group = bind_group(
            device,
            &layout,
            &globals,
            &lut_view,
            &sampler,
            (&grid, &slice_uniform),
            (&iso_grid, &iso_uniform),
        );
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
        let face_attrs = wgpu::vertex_attr_array![
            0 => Float32x3,
            1 => Float32x3,
            2 => Float32x3,
            3 => Float32x4
        ];
        let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let face_pipeline = pipeline(
            "murmel-faces",
            "vs_face",
            "fs_face",
            &[Some(wgpu::VertexBufferLayout {
                array_stride: FACE_STRIDE,
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
        let outline_pipeline = pipeline(
            "murmel-outline",
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
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            },
        );
        // A segment is two line vertices (position, colour) in a row: as an
        // instance, the first vertex at offset 0, the second at 24.
        let wide_attrs = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x3
        ];
        let wide_layout = wgpu::VertexBufferLayout {
            array_stride: 48,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wide_attrs,
        };
        let wide_pipeline = pipeline(
            "murmel-wide-edges",
            "vs_wide",
            "fs_line",
            &[Some(wide_layout.clone())],
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            },
        );
        let wide_outline_pipeline = pipeline(
            "murmel-wide-outline",
            "vs_wide",
            "fs_line",
            &[Some(wide_layout)],
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
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
        let slice_pipeline = pipeline(
            "murmel-slice-preview",
            "vs_face",
            "fs_slice",
            &[Some(wgpu::VertexBufferLayout {
                array_stride: FACE_STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &face_attrs,
            })],
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 1.5,
                    clamp: 0.0,
                },
            },
        );
        // The iso preview draws the back faces of its grid's box (so the
        // eye may be inside it) and writes the depth of the surface found.
        let box_attrs = wgpu::vertex_attr_array![0 => Float32x3];
        let iso_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("murmel-iso-preview"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_box"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &box_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Front),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLES,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_iso"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            globals,
            lut,
            lut_key: None,
            bind_group,
            iso_pipeline,
            iso_uniform,
            iso_grid,
            iso_id: None,
            slice_pipeline,
            layout,
            lut_view,
            sampler,
            slice_uniform,
            grid,
            grid_id: None,
            face_pipeline,
            line_pipeline,
            outline_pipeline,
            wide_pipeline,
            wide_outline_pipeline,
            bg_pipeline,
            geometry: [None, None],
            mesh_key: None,
            layer: None,
            layer_revision: None,
            revision: None,
            targets: None,
            piece: PIECE_BYTES.min(device.limits().max_buffer_size),
        }
    }

    /// Upload `surface`: its extras alone when its own faces are those
    /// uploaded already (the same [`murmel_engine::surface::BuildKey`]),
    /// else both parts.
    fn upload(&mut self, device: &wgpu::Device, surface: Option<&Surface>) {
        let Some(s) = surface else {
            self.geometry = [None, None];
            self.mesh_key = None;
            return;
        };
        if self.mesh_key != Some(s.key.0) {
            self.geometry[0] = geometry(device, s, Part::Mesh, self.piece);
            self.mesh_key = Some(s.key.0);
        }
        self.geometry[1] = geometry(device, s, Part::Extras, self.piece);
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
        match view.layer {
            Some((s, r)) if self.layer_revision != Some(r) => {
                self.layer = geometry(device, s, Part::Mesh, self.piece);
                self.layer_revision = Some(r);
            }
            None => {
                self.layer = None;
                self.layer_revision = None;
            }
            _ => {}
        }
        self.ensure_targets(device, size);
        let (origin, radius) = view
            .surface
            .map_or(([0.0; 3], 1.0), |s| (s.origin, s.radius));
        let frame = view
            .camera
            .frame(origin, radius, size[0] as f64 / size[1] as f64);
        let f = view.field;
        if let Some(p) = f
            && self.lut_key != Some((p.map, p.reverse))
        {
            self.write_lut(queue, p.map, p.reverse);
        }
        let g = Globals {
            view_proj: frame.view_proj.to_cols_array_2d(),
            eye: f32x3(frame.eye),
            to_eye: f32x3(frame.light),
            params: [
                if frame.orthographic { 1.0 } else { 0.0 },
                view.line_width.max(1.0),
                size[0] as f32,
                size[1] as f32,
            ],
            bg_top: f32x3(view.bg_top),
            bg_bottom: f32x3(view.bg_bottom),
            field: f.map_or([0.0, 1.0, 1.0, 0.0], |p| {
                [p.lo, p.hi, p.bands as f32, p.lines as f32]
            }),
            field_mode: f.map_or([0.0; 4], |p| {
                let mode = match p.mode {
                    murmel_engine::FieldMode::Flat => 1.0,
                    murmel_engine::FieldMode::Smooth => 2.0,
                    murmel_engine::FieldMode::Banded => 3.0,
                };
                // Lines keep their width in pixels on a HiDPI target.
                [mode, p.line_mix, LINE_PX * view.scale.max(1.0), 0.0]
            }),
            contour: f.map_or([0.0; 4], |p| f32x3(p.line_color)),
            selection: f.map_or([0.0; 4], |p| f32x3(p.selection)),
            clip_planes: std::array::from_fn(|i| {
                view.clip_planes.get(i).copied().unwrap_or([0.0; 4])
            }),
            clip_count: [
                view.clip_planes.len().min(MAX_PREVIEW_PLANES) as f32,
                0.0,
                0.0,
                0.0,
            ],
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        let [planes, iso_box] = self.prepare_previews(device, queue, view, frame.to_eye);
        // What the slice preview leaves out of the extras, as ranges of
        // their own buffers: the moving slices where they were built.
        let at = view.surface.map(|s| s.extras_at);
        let hide_faces = murmel_engine::slice_preview::merged(
            view.slice
                .iter()
                .flat_map(|d| d.hide_faces.iter().cloned())
                .chain(view.iso.iter().flat_map(|d| d.hide_faces.iter().cloned()))
                .collect(),
        );
        let shown = |hide: &[std::ops::Range<u32>], base: usize, len: u32| {
            let base = base as u32;
            let hide: Vec<std::ops::Range<u32>> = hide
                .iter()
                .filter(|r| r.end > base)
                .map(|r| r.start.max(base) - base..r.end - base)
                .collect();
            murmel_engine::slice_preview::shown_ranges(len, &hide)
        };
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
                        load: wgpu::LoadOp::Clear(if view.transparent {
                            wgpu::Color::TRANSPARENT
                        } else {
                            wgpu::Color::BLACK
                        }),
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
            if !view.transparent {
                pass.set_pipeline(&self.bg_pipeline);
                pass.draw(0..3, 0..1);
            }
            // One-pixel lines keep the line list, so the window and the
            // render tests draw exactly as before.
            let wide = view.line_width > 1.0;
            // Faces, then edges, then streamlines, then the outline, each
            // the mesh's first and then the extras', as one buffer drew
            // them.
            let parts = || self.geometry.iter().flatten();
            // The ranges of each part drawn: all of the mesh's; of the
            // extras, what a slice preview does not leave out.
            let ranges = |k: usize, len: u32, faces: bool| match (k, view.slice, at) {
                (1, _, Some(at)) if faces => shown(&hide_faces, at.indices, len),
                (1, Some(d), Some(at)) => shown(&d.hide_edges, at.edges, len),
                _ => std::iter::once(0..len).collect(),
            };
            if view.faces {
                for (k, geo) in self.geometry.iter().enumerate() {
                    let Some(geo) = geo.as_ref().filter(|g| g.index_count > 0) else {
                        continue;
                    };
                    pass.set_pipeline(&self.face_pipeline);
                    for r in ranges(k, geo.index_count, true) {
                        for (p, r) in within(&geo.faces, |p| p.at.clone(), r) {
                            pass.set_vertex_buffer(0, p.vertices.slice(..));
                            pass.set_index_buffer(p.indices.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(r, 0, 0..1);
                        }
                    }
                }
                // The thin layer as instance 1, which the preview planes
                // leave uncut.
                if let Some(geo) = self.layer.as_ref().filter(|g| g.index_count > 0) {
                    pass.set_pipeline(&self.face_pipeline);
                    for p in &geo.faces {
                        pass.set_vertex_buffer(0, p.vertices.slice(..));
                        pass.set_index_buffer(p.indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..p.at.len() as u32, 0, 1..2);
                    }
                }
                if let Some((buf, count)) = &planes {
                    pass.set_pipeline(&self.slice_pipeline);
                    pass.set_vertex_buffer(0, buf.slice(..));
                    pass.draw(0..*count, 0..1);
                }
                if let Some((buf, count)) = &iso_box {
                    pass.set_pipeline(&self.iso_pipeline);
                    pass.set_vertex_buffer(0, buf.slice(..));
                    pass.draw(0..*count, 0..1);
                }
            }
            let lines = |pass: &mut wgpu::RenderPass<'_>,
                         pieces: &[LinePiece],
                         count: std::ops::Range<u32>,
                         narrow: &wgpu::RenderPipeline,
                         widened: &wgpu::RenderPipeline| {
                for (p, count) in within(pieces, |p| p.at.clone(), count) {
                    pass.set_vertex_buffer(0, p.vertices.slice(..));
                    if wide {
                        pass.set_pipeline(widened);
                        pass.draw(0..6, count.start / 2..count.end / 2);
                    } else {
                        pass.set_pipeline(narrow);
                        pass.draw(count, 0..1);
                    }
                }
            };
            if view.edges {
                for (k, geo) in self.geometry.iter().enumerate() {
                    let Some(geo) = geo else { continue };
                    for r in ranges(k, geo.edge_count, false) {
                        let (n, w) = (&self.line_pipeline, &self.wide_pipeline);
                        lines(&mut pass, &geo.edges, r, n, w);
                    }
                }
                // The layer's edges as instance 1 of one-pixel lines (a
                // clip drag is only drawn in the window; pictures settle
                // the exact cut first).
                if let Some(geo) = &self.layer {
                    pass.set_pipeline(&self.line_pipeline);
                    for p in &geo.edges {
                        pass.set_vertex_buffer(0, p.vertices.slice(..));
                        pass.draw(0..p.at.len() as u32, 1..2);
                    }
                }
            }
            for geo in parts() {
                let (n, w) = (&self.line_pipeline, &self.wide_pipeline);
                lines(&mut pass, &geo.streams, 0..geo.stream_count, n, w);
            }
            for geo in parts() {
                let (n, w) = (&self.outline_pipeline, &self.wide_outline_pipeline);
                lines(&mut pass, &geo.outline, 0..geo.outline_count, n, w);
            }
        }
        queue.submit([encoder.finish()]);
    }

    /// Fill the colour table with colour map `id`, turned when `reverse`.
    fn write_lut(&mut self, queue: &wgpu::Queue, id: &'static str, reverse: bool) {
        let Some(map) = murmel_engine::colormap::get(id) else {
            return;
        };
        let texels = map.lut(reverse);
        let n = texels.len() as u32;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.lut,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * n),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: n,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        self.lut_key = Some((id, reverse));
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
    /// RGBA8 rows, sRGB, straight (not premultiplied) alpha.
    pub rgba: Vec<u8>,
    pub adapter: String,
}

/// How an offscreen picture is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shoot {
    pub width: u32,
    pub height: u32,
    /// Leave the background out: alpha 0 where no face or line is.
    pub transparent: bool,
    /// Width of edge lines in pixels (1 as in the window).
    pub line_width: f32,
}

impl Shoot {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            transparent: false,
            line_width: 1.0,
        }
    }
}

/// A renderer on the shared offscreen device, for one picture or the
/// frames of a movie (the mesh is uploaded once per surface revision).
pub struct Offscreen<'g> {
    gpu: &'g Gpu,
    renderer: MeshRenderer,
}

impl Offscreen<'_> {
    /// Render `session` as `shoot` says and read the picture back.
    pub fn shoot(
        &mut self,
        session: &murmel_engine::Session,
        shoot: &Shoot,
    ) -> Result<Shot, RenderError> {
        let (width, height) = (shoot.width, shoot.height);
        if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
            return Err(RenderError::new(
                "RENDER003",
                format!("{width} x {height} is outside 1..{MAX_SIDE}"),
            ));
        }
        let dark = session.theme.is_dark(None);
        let p = session.tokens.palette(dark);
        let field = session.field_paint();
        // What the window shows: during a clip preview, the surface cut on
        // the GPU (tools settle it first, so their pictures are exact).
        let drawn = session.drawn();
        let (surface, revision, planes, layer) = match &drawn {
            Some(d) => (Some(d.surface), d.revision, d.planes.as_slice(), d.layer),
            None => (None, session.surface_revision, &[][..], None),
        };
        let slice = session.slice_drawn();
        let iso = session.iso_drawn();
        self.renderer.render(
            &self.gpu.device,
            &self.gpu.queue,
            &View {
                surface,
                revision,
                camera: &session.camera,
                size: [width, height],
                bg_top: p.viewport_top.linear(),
                bg_bottom: p.viewport_bottom.linear(),
                faces: session.display.draw_faces(),
                edges: session.display.draw_edges(),
                line_width: shoot.line_width,
                transparent: shoot.transparent,
                field: field.as_ref(),
                // Contour lines widen with the edges of a scaled export.
                scale: shoot.line_width,
                clip_planes: planes,
                layer,
                slice: slice.as_ref(),
                iso: iso.as_ref(),
            },
        );
        let mut rgba = self.renderer.read_rgba(&self.gpu.device, &self.gpu.queue)?;
        if shoot.transparent {
            unpremultiply_linear(&mut rgba);
        }
        Ok(Shot {
            width,
            height,
            rgba,
            adapter: self.gpu.adapter.clone(),
        })
    }
}

/// Run `f` with a renderer on the shared offscreen device, which stays
/// locked meanwhile (one offscreen job at a time).
pub fn with_offscreen<R>(f: impl FnOnce(&mut Offscreen<'_>) -> R) -> Result<R, RenderError> {
    let gpu = Gpu::shared()?;
    let gpu = gpu
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut o = Offscreen {
        renderer: MeshRenderer::new(&gpu.device),
        gpu: &gpu,
    };
    Ok(f(&mut o))
}

/// Render `session` at `width` × `height` on a headless GPU.
pub fn render_session(
    session: &murmel_engine::Session,
    width: u32,
    height: u32,
) -> Result<Shot, RenderError> {
    render_session_with(session, &Shoot::new(width, height))
}

/// Render `session` as `shoot` says on a headless GPU.
pub fn render_session_with(
    session: &murmel_engine::Session,
    shoot: &Shoot,
) -> Result<Shot, RenderError> {
    if !(1..=MAX_SIDE).contains(&shoot.width) || !(1..=MAX_SIDE).contains(&shoot.height) {
        return Err(RenderError::new(
            "RENDER003",
            format!(
                "{} x {} is outside 1..{MAX_SIDE}",
                shoot.width, shoot.height
            ),
        ));
    }
    with_offscreen(|o| o.shoot(session, shoot))?
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// The GPU resolves multisampled edges of a transparent picture to
/// colours premultiplied in linear light and stored as sRGB; PNG wants
/// straight alpha. Divide by alpha in linear light where 0 < alpha < 1.
pub fn unpremultiply_linear(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3];
        if a == 0 || a == 255 {
            continue;
        }
        let af = a as f32 / 255.0;
        for c in &mut px[..3] {
            let lin = srgb_to_linear(*c as f32 / 255.0) / af;
            *c = (linear_to_srgb(lin.min(1.0)) * 255.0).round() as u8;
        }
    }
}

/// Encode RGBA8 rows as a PNG file.
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), RenderError> {
    write_png_with(path, width, height, rgba, None)
}

/// Encode RGBA8 rows as a PNG file, with the resolution `dpi` stored in
/// its pHYs chunk when given (print size = pixels / dpi).
pub fn write_png_with(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    dpi: Option<f64>,
) -> Result<(), RenderError> {
    let file = std::fs::File::create(path)
        .map_err(|e| RenderError::new("RENDER004", format!("{}: {e}", path.display())))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    if let Some(d) = dpi.filter(|d| d.is_finite() && *d > 0.0) {
        let ppm = (d / 0.0254).round() as u32;
        enc.set_pixel_dims(Some(png::PixelDimensions {
            xppu: ppm,
            yppu: ppm,
            unit: png::Unit::Meter,
        }));
    }
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
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > tol))
        .count();
    bad as f64 / (a.len() / 4) as f64
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// Keep rule: a surface drawn in pieces draws the picture of the same
    /// surface in one buffer each, with one-pixel and widened lines. A
    /// piece that drops or doubles a triangle, numbers its vertices from
    /// the wrong first one, or splits an edge between two pieces moves
    /// pixels here. The render tests draw every fixture in one piece (far
    /// below a GPU buffer), and only a mesh of tens of millions of cells
    /// needs several, so no other test draws pieces.
    /// Keep rule: of the GPU errors before the window shows one, the
    /// first is shown (the cause; those after it mostly follow from it),
    /// once. Keeping the last instead, or showing one again on every
    /// frame, fails here; the window has no GPU in its tests.
    #[test]
    fn the_first_gpu_error_is_shown_once() {
        let errors = GpuErrors::default();
        let e = |what: &str| wgpu::Error::Validation {
            source: Box::new(std::io::Error::other(what.to_string())),
            description: what.to_string(),
        };
        assert_eq!(errors.take(), None);
        errors.keep(&e("buffer too large"));
        errors.keep(&e("buffer is invalid"));
        let first = errors.take().unwrap();
        assert_eq!(first.code, "RENDER006");
        assert!(first.message.contains("buffer too large"), "{first:?}");
        assert_eq!(errors.take(), None);
    }

    #[test]
    fn a_surface_in_pieces_draws_the_same_picture() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut s = murmel_engine::Session::new(murmel_engine::Tokens::builtin().unwrap());
        s.viewport = [320.0, 240.0];
        s.open(&root.join("test-fixtures/vtkhdf/channel_fields.vtkhdf"))
            .unwrap();
        s.camera.set_standard(murmel_engine::StandardView::ISO);
        s.fit();
        // A slice, so the extras are drawn in pieces too; the mesh shows
        // its outline meanwhile.
        let b = s.mesh.as_ref().unwrap().mesh.bounds();
        s.display.slices = vec![murmel_engine::Slice::new(b.center(), [1.0, 0.0, 0.0])];
        s.redraw();
        let gpu = Gpu::shared().unwrap();
        let gpu = gpu
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pieces = |r: &MeshRenderer| -> Vec<(usize, usize)> {
            r.geometry
                .iter()
                .flatten()
                .map(|g| (g.faces.len(), g.edges.len()))
                .collect()
        };
        for line_width in [1.0, 2.0] {
            let shoot = Shoot {
                line_width,
                ..Shoot::new(320, 240)
            };
            let mut one = Offscreen {
                renderer: MeshRenderer::new(&gpu.device),
                gpu: &gpu,
            };
            let a = one.shoot(&s, &shoot).unwrap();
            let mut split = Offscreen {
                renderer: MeshRenderer::new(&gpu.device),
                gpu: &gpu,
            };
            split.renderer.piece = 4096;
            let b = split.shoot(&s, &shoot).unwrap();
            // The mesh as its faint outline (lines only) and the slice
            // (faces and edges): one piece each, or many.
            assert_eq!(pieces(&one.renderer), [(0, 1), (1, 1)]);
            let p = pieces(&split.renderer);
            assert!(
                p[0].0 == 0 && p[0].1 > 1 && p[1].0 > 1 && p[1].1 > 1,
                "{p:?}"
            );
            let differ = a.rgba.iter().zip(&b.rgba).filter(|(x, y)| x != y).count();
            assert_eq!(differ, 0, "line width {line_width}");
        }
    }
}
