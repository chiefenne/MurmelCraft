// Shaded with edges (UI spec, Display). Colours arrive linear; the target
// is sRGB. Lighting is a key light near the eye, two-sided, so face orientation never
// darkens a face.

struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    to_eye: vec4<f32>,
    // x: 1 for orthographic
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
