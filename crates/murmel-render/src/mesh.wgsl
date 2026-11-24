// Shaded with edges (UI spec, Display). Colours arrive linear; the target
// is sRGB. Lighting is a key light near the eye, two-sided, so face orientation never
// darkens a face.

struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    to_eye: vec4<f32>,
    // x: 1 for orthographic, y: width of wide lines in pixels, zw: the
    // size of the picture in pixels
    params: vec4<f32>,
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct FaceIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

struct FaceOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

@vertex
fn vs_face(v: FaceIn) -> FaceOut {
    var o: FaceOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.world = v.pos;
    o.normal = v.normal;
    o.color = v.color;
    return o;
}

@fragment
fn fs_face(f: FaceOut) -> @location(0) vec4<f32> {
    // to_eye carries the key light direction (from the eye, a little above
    // and to the left), the same for both projections.
    let l = g.to_eye.xyz;
    let d = abs(dot(normalize(f.normal), l));
    let shade = 0.32 + 0.68 * d;
    return vec4<f32>(f.color * shade, 1.0);
}

struct LineIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec3<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_line(v: LineIn) -> LineOut {
    var o: LineOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.color = v.color;
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
    }
    let off = (n * sides[i] + d * (end * 2.0 - 1.0)) * half_width / half_size;
    o.clip = vec4<f32>(c.xy + off * c.w, c.z, c.w);
    return o;
}

@fragment
fn fs_line(f: LineOut) -> @location(0) vec4<f32> {
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
