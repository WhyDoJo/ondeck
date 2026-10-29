struct Camera {
    center: vec2<f32>,
    zoom: f32,
    scale: f32,
    viewport_px: vec2<f32>,
    _pad: vec2<f32>,
}

struct VsIn {
    @location(0) corner: vec2<f32>,
    @location(1) a: vec2<f32>,
    @location(2) b: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) params: vec4<f32>,
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) screen: vec2<f32>,
    @location(1) a_s: vec2<f32>,
    @location(2) b_s: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) half_px: f32,
    @location(5) kind: f32,
}

@group(0) @binding(0) var<uniform> cam: Camera;

fn to_screen(world: vec2<f32>) -> vec2<f32> {
    let view = cam.viewport_px / cam.scale;
    let logical = (world - cam.center) * cam.zoom + view * 0.5;
    return logical * cam.scale;
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let kind = in.params.y;
    let a_s = to_screen(in.a);
    let b_s = to_screen(in.b);
    var half_px = in.params.x * cam.zoom * cam.scale;
    if (kind < 1.5) {
        half_px = max(half_px, 0.7);
    }
    let aa = 1.6;
    var screen: vec2<f32>;
    if (kind < 0.5) {
        let delta = b_s - a_s;
        let len = max(length(delta), 0.001);
        let dir = delta / len;
        let n = vec2<f32>(-dir.y, dir.x);
        let center = (a_s + b_s) * 0.5;
        let along = len * 0.5 + half_px + aa;
        let across = half_px + aa;
        screen = center + dir * in.corner.x * along + n * in.corner.y * across;
    } else {
        let pad = select(half_px + aa, half_px + 2.4, kind > 1.5);
        screen = a_s + in.corner * pad;
    }
    let ndc = vec2<f32>(
        screen.x / cam.viewport_px.x * 2.0 - 1.0,
        1.0 - screen.y / cam.viewport_px.y * 2.0,
    );
    var out: VsOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.screen = screen;
    out.a_s = a_s;
    out.b_s = b_s;
    out.color = in.color;
    out.half_px = half_px;
    out.kind = kind;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var alpha = 0.0;
    if (in.kind > 1.5) {
        let d = abs(length(in.screen - in.a_s) - in.half_px);
        alpha = 1.0 - smoothstep(0.65, 1.45, d);
    } else {
        var dist = 0.0;
        if (in.kind > 0.5) {
            dist = length(in.screen - in.a_s);
        } else {
            let ba = in.b_s - in.a_s;
            let denom = max(dot(ba, ba), 1e-4);
            let h = clamp(dot(in.screen - in.a_s, ba) / denom, 0.0, 1.0);
            dist = length(in.screen - in.a_s - ba * h);
        }
        alpha = 1.0 - smoothstep(in.half_px - 0.2, in.half_px + 1.05, dist);
    }
    alpha = alpha * in.color.a;
    if (alpha < 0.004) {
        discard;
    }
    return vec4<f32>(in.color.rgb, alpha);
}
