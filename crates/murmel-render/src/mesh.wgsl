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
    // x: 1 for orthographic
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
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var lut: texture_2d<f32>;
@group(0) @binding(2) var lut_sampler: sampler;

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
    var color = f.color;
    if (g.field_mode.x > 0.5 && f.value.w > 0.5) {
        var t = tn;
        if (g.field_mode.x < 1.5) {
            t = (f.value.x - g.field.x) / span;
        }
        t = clamp(t, 0.0, 1.0);
        if (g.field_mode.x > 2.5) {
            let n = max(g.field.z, 1.0);
            t = (min(floor(t * n), n - 1.0) + 0.5) / n;
        }
        let u = (t * 255.0 + 0.5) / 256.0;
        var c = textureSampleLevel(lut, lut_sampler, vec2<f32>(u, 0.5), 0.0).rgb;
        c = mix(c, g.selection.xyz, f.value.z);
        if (lines > 0.5) {
            let k = round(x);
            let px = abs(x - k) / max(w, 1e-6);
            let half = 0.5 * g.field_mode.z;
            let a = 1.0 - smoothstep(half - 0.5, half + 0.5, px);
            if (k >= 1.0 && k <= lines) {
                c = mix(c, g.contour.xyz, a * g.field_mode.y);
            }
        }
        color = c;
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
