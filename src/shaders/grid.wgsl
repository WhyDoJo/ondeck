struct Camera {
    center: vec2<f32>,
    zoom: f32,
    scale: f32,
    viewport_px: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> cam: Camera;

fn linear_channel(s: f32) -> f32 {
    if (s <= 0.04045) {
        return s / 12.92;
    }
    return pow((s + 0.055) / 1.055, 2.4);
}

fn linear_rgb(r: f32, g: f32, b: f32) -> vec3<f32> {
    return vec3<f32>(linear_channel(r), linear_channel(g), linear_channel(b));
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let paper = linear_rgb(0.953, 0.941, 0.910);
    let dot = linear_rgb(0.820, 0.796, 0.753);
    let logical = frag.xy / cam.scale;
    let view = cam.viewport_px / cam.scale;
    let world = (logical - view * 0.5) / max(cam.zoom, 0.0001) + cam.center;
    let spacing = 32.0;
    let spacing_px = spacing * cam.zoom * cam.scale;
    let fade = smoothstep(8.0, 14.0, spacing_px);
    let gp = round(world / spacing) * spacing;
    let dist_px = length((world - gp) * cam.zoom * cam.scale);
    let dot_a = (1.0 - smoothstep(1.05, 1.85, dist_px)) * fade * 0.9;
    return vec4<f32>(mix(paper, dot, dot_a), 1.0);
}
