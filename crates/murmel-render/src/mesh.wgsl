// Shaded with edges (UI spec, Display). Colours arrive linear; the target
// is sRGB. Lighting is a key light near the eye, two-sided, so face orientation never
// darkens a face.
//
// Colour mode field: each face vertex carries the cell value and the node
// value of the field (relative to the surface's value origin); the
// fragment maps one of them through the colour table `lut` over the range
// in `g.field`. Banded colouring takes the band's middle colour; contour
// lines are drawn where the node value crosses one of `lines` even steps,
// anti-aliased by the screen-space rate of change of the value (fwidth),
// so they stay about `g.field_mode.z` pixels wide at any zoom.

struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    to_eye: vec4<f32>,
    // x: 1 for orthographic, y: width of wide lines in pixels, zw: the
    // size of the picture in pixels
    params: vec4<f32>,
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
    // x lo, y hi, z bands, w contour lines (0: none)
    field: vec4<f32>,
    // x mode (0 off, 1 flat, 2 smooth, 3 banded), y line mix, z line width
    // in pixels
    field_mode: vec4<f32>,
    contour: vec4<f32>,
    selection: vec4<f32>,
    // The clip planes of a preview (the window's clip drag), relative to
    // the surface's origin: xyz the normal, w the offset; a point p is cut
    // away where dot(n, p) + w > 0. clip_count.x of them are in use.
    clip_planes: array<vec4<f32>, 8>,
    clip_count: vec4<f32>,
};

// Whether a preview plane cuts the point away.
fn cut_away(p: vec3<f32>) -> bool {
    let n = u32(g.clip_count.x);
    for (var i = 0u; i < n; i = i + 1u) {
        let c = g.clip_planes[i];
        if (dot(c.xyz, p) + c.w > 0.0) {
            return true;
        }
    }
    return false;
}

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var lut: texture_2d<f32>;
@group(0) @binding(2) var lut_sampler: sampler;

// The preview of a slice dragged in the window (murmel_engine's
// slice_preview): the field on a regular grid, per grid cell the cell
// value (x) and the value the cell's gradient predicts at the grid
// cell's centre (y), relative to the value origin; x above 1e38 outside
// the mesh, below -1e38 for a cell without a value.
struct SliceGrid {
    // xyz where grid cell (0, 0, 0) starts, relative to the surface's
    // origin; w 1 when the planes are coloured from the grid
    lo: vec4<f32>,
    side: vec4<f32>,
    // xyz the grid cells along each axis; w the clip planes in use
    dims: vec4<f32>,
    // The clip planes that cut the slices, as clip_planes.
    clips: array<vec4<f32>, 8>,
};

@group(0) @binding(3) var grid: texture_3d<f32>;
@group(0) @binding(4) var<uniform> sg: SliceGrid;

struct FaceIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // x cell value, y node value, z selection mix, w 1 with a value
    @location(3) value: vec4<f32>,
};

struct FaceOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) value: vec4<f32>,
};

@vertex
fn vs_face(v: FaceIn) -> FaceOut {
    var o: FaceOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.world = v.pos;
    o.normal = v.normal;
    o.color = v.color;
    o.value = v.value;
    return o;
}

@fragment
fn fs_face(f: FaceOut) -> @location(0) vec4<f32> {
    // to_eye carries the key light direction (from the eye, a little above
    // and to the left), the same for both projections.
    let l = g.to_eye.xyz;
    let d = abs(dot(normalize(f.normal), l));
    let shade = 0.32 + 0.68 * d;
    // The node value in steps of the contour lines; its rate of change on
    // screen is taken here, before any branch, as derivatives need.
    let span = max(g.field.y - g.field.x, 1e-30);
    let tn = (f.value.y - g.field.x) / span;
    let lines = g.field.w;
    let x = tn * (lines + 1.0);
    let w = fwidth(x);
    if (cut_away(f.world)) {
        discard;
    }
    var color = f.color;
    if (g.field_mode.x > 0.5 && f.value.w > 0.5) {
        color = field_color(f.value.x, tn, x, w, f.value.z);
    }
    return vec4<f32>(color * shade, 1.0);
}

// The colour of a field value: the cell value `cell` (colour mode flat)
// or the node value in steps of the range `tn` (smooth and banded), with
// the contour lines (`x` the node value in steps of the lines, `w` its
// rate of change on screen) and `sel` of the selection colour mixed in.
fn field_color(cell: f32, tn: f32, x: f32, w: f32, sel: f32) -> vec3<f32> {
    let span = max(g.field.y - g.field.x, 1e-30);
    let lines = g.field.w;
    var t = tn;
    if (g.field_mode.x < 1.5) {
        t = (cell - g.field.x) / span;
    }
    t = clamp(t, 0.0, 1.0);
    if (g.field_mode.x > 2.5) {
        let n = max(g.field.z, 1.0);
        t = (min(floor(t * n), n - 1.0) + 0.5) / n;
    }
    let u = (t * 255.0 + 0.5) / 256.0;
    var c = textureSampleLevel(lut, lut_sampler, vec2<f32>(u, 0.5), 0.0).rgb;
    c = mix(c, g.selection.xyz, sel);
    if (lines > 0.5) {
        let k = round(x);
        let px = abs(x - k) / max(w, 1e-6);
        let half = 0.5 * g.field_mode.z;
        let a = 1.0 - smoothstep(half - 0.5, half + 0.5, px);
        if (k >= 1.0 && k <= lines) {
            c = mix(c, g.contour.xyz, a * g.field_mode.y);
        }
    }
    return c;
}

fn grid_at(i: vec3<i32>) -> vec2<f32> {
    return textureLoad(grid, i, 0).xy;
}

// A plane of a slice being dragged, coloured from the field's grid: the
// cell value of the grid cell the fragment is in (colour mode flat), or
// the predicted values mixed between the eight nearest grid cell centres
// (smooth and banded); nothing outside the mesh. Without the grid, the
// slice's own colour.
@fragment
fn fs_slice(f: FaceOut) -> @location(0) vec4<f32> {
    let l = g.to_eye.xyz;
    let d = abs(dot(normalize(f.normal), l));
    let shade = 0.32 + 0.68 * d;
    let dims = max(vec3<i32>(sg.dims.xyz), vec3<i32>(1));
    let top = dims - vec3<i32>(1);
    let q = (f.world - sg.lo.xyz) / max(sg.side.xyz, vec3<f32>(1e-30));
    let here = grid_at(clamp(vec3<i32>(floor(q)), vec3<i32>(0), top));
    let t0 = q - vec3<f32>(0.5);
    let b = vec3<i32>(floor(t0));
    let fr = t0 - floor(t0);
    var sum = 0.0;
    var wsum = 0.0;
    for (var k = 0; k < 8; k = k + 1) {
        let o = vec3<i32>(k & 1, (k >> 1u) & 1, (k >> 2u) & 1);
        let t = grid_at(clamp(b + o, vec3<i32>(0), top));
        let wk = select(1.0 - fr.x, fr.x, o.x == 1)
            * select(1.0 - fr.y, fr.y, o.y == 1)
            * select(1.0 - fr.z, fr.z, o.z == 1);
        if (abs(t.x) < 1e38) {
            sum = sum + wk * t.y;
            wsum = wsum + wk;
        }
    }
    var v = here.y;
    if (wsum > 1e-6) {
        v = sum / wsum;
    }
    let span = max(g.field.y - g.field.x, 1e-30);
    let tn = (v - g.field.x) / span;
    let x = tn * (g.field.w + 1.0);
    let w = fwidth(x);
    let colored = sg.lo.w > 0.5;
    let inside = all(q >= vec3<f32>(0.0)) && all(q < vec3<f32>(dims));
    if (colored && (!inside || here.x > 1e38)) {
        discard;
    }
    let n = u32(sg.dims.w);
    for (var i = 0u; i < n; i = i + 1u) {
        let c = sg.clips[i];
        if (dot(c.xyz, f.world) + c.w > 0.0) {
            discard;
        }
    }
    var color = f.color;
    if (colored && g.field_mode.x > 0.5 && here.x > -1e38) {
        color = field_color(here.x, tn, x, w, f.value.z);
    }
    return vec4<f32>(color * shade, 1.0);
}

struct LineIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec3<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) world: vec3<f32>,
};

@vertex
fn vs_line(v: LineIn) -> LineOut {
    var o: LineOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.color = v.color;
    o.world = v.pos;
    return o;
}

// Lines wider than a pixel (exported pictures at a scale): each segment
// is an instance of six vertices, a quad around the segment on screen,
// `params.y` pixels wide and extended by half the width past both ends so
// joints close. The segment is first cut at the near plane (z = 0 in clip
// space) so a line through the camera still widens correctly.
struct WideIn {
    @location(0) p0: vec3<f32>,
    @location(1) c0: vec3<f32>,
    @location(2) p1: vec3<f32>,
    @location(3) c1: vec3<f32>,
};

@vertex
fn vs_wide(v: WideIn, @builtin(vertex_index) i: u32) -> LineOut {
    var ends = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    var sides = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    var a = g.view_proj * vec4<f32>(v.p0, 1.0);
    var b = g.view_proj * vec4<f32>(v.p1, 1.0);
    var o: LineOut;
    o.color = v.c0;
    o.world = v.p0;
    if (a.z < 0.0 && b.z < 0.0) {
        // Wholly behind the camera: a degenerate triangle.
        o.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        return o;
    }
    if (a.z < 0.0) {
        a = mix(a, b, a.z / (a.z - b.z));
    } else if (b.z < 0.0) {
        b = mix(b, a, b.z / (b.z - a.z));
    }
    let half_size = 0.5 * g.params.zw;
    let sa = a.xy / a.w * half_size;
    let sb = b.xy / b.w * half_size;
    let d0 = sb - sa;
    let len = length(d0);
    var d = vec2<f32>(1.0, 0.0);
    if (len > 1e-6) {
        d = d0 / len;
    }
    let n = vec2<f32>(-d.y, d.x);
    let half_width = 0.5 * g.params.y;
    let end = ends[i];
    var c = a;
    if (end > 0.5) {
        c = b;
        o.color = v.c1;
        o.world = v.p1;
    }
    let off = (n * sides[i] + d * (end * 2.0 - 1.0)) * half_width / half_size;
    o.clip = vec4<f32>(c.xy + off * c.w, c.z, c.w);
    return o;
}

@fragment
fn fs_line(f: LineOut) -> @location(0) vec4<f32> {
    if (cut_away(f.world)) {
        discard;
    }
    return vec4<f32>(f.color, 1.0);
}

struct BgOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) t: f32,
};

// One triangle that covers the viewport; t runs 0 at the top to 1 at the
// bottom.
@vertex
fn vs_bg(@builtin(vertex_index) i: u32) -> BgOut {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    var o: BgOut;
    o.clip = vec4<f32>(x, y, 1.0, 1.0);
    o.t = 0.5 - 0.5 * y;
    return o;
}

@fragment
fn fs_bg(f: BgOut) -> @location(0) vec4<f32> {
    return vec4<f32>(mix(g.bg_top.xyz, g.bg_bottom.xyz, clamp(f.t, 0.0, 1.0)), 1.0);
}
