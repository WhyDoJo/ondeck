struct VsIn {
    @location(0) clip: vec2<f32>,
    @location(1) logical: vec2<f32>,
    @location(2) rect: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) extra: vec4<f32>,
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) logical: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) extra: vec4<f32>,
}

fn rrect(p: vec2<f32>, rect: vec4<f32>, radius: f32) -> f32 {
    let c = rect.xy + rect.zw * 0.5;
    let h = rect.zw * 0.5 - vec2<f32>(radius);
    let q = abs(p - c) - h;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn capsule(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-4), 0.0, 1.0);
    return length(pa - ba * h) - r;
}

fn rot(p: vec2<f32>, a: f32) -> vec2<f32> {
    let s = sin(a);
    let c = cos(a);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

fn icon_dist(id: f32, p: vec2<f32>) -> f32 {
    if (id < 1.5) {
        let q = rot(p, 0.62);
        let body = capsule(q, vec2<f32>(-7.5, 0.0), vec2<f32>(6.5, 0.0), 1.55);
        let tail = capsule(q, vec2<f32>(-7.4, -2.4), vec2<f32>(-7.4, 2.4), 1.35);
        return min(body, tail);
    }
    if (id < 2.5) {
        let q = rot(p, -0.45);
        let box = rrect(q, vec4<f32>(-7.0, -4.6, 14.0, 9.2), 1.6);
        return box;
    }
    if (id < 3.5) {
        let shaft = capsule(p, vec2<f32>(6.5, 0.0), vec2<f32>(-3.2, 0.0), 1.4);
        let h1 = capsule(p, vec2<f32>(-3.2, 0.0), vec2<f32>(2.2, 5.0), 1.4);
        let h2 = capsule(p, vec2<f32>(-3.2, 0.0), vec2<f32>(2.2, -5.0), 1.4);
        return min(shaft, min(h1, h2));
    }
    if (id < 4.5) {
        let q = vec2<f32>(-p.x, p.y);
        let shaft = capsule(q, vec2<f32>(6.5, 0.0), vec2<f32>(-3.2, 0.0), 1.4);
        let h1 = capsule(q, vec2<f32>(-3.2, 0.0), vec2<f32>(2.2, 5.0), 1.4);
        let h2 = capsule(q, vec2<f32>(-3.2, 0.0), vec2<f32>(2.2, -5.0), 1.4);
        return min(shaft, min(h1, h2));
    }
    let lid = capsule(p, vec2<f32>(-6.2, -5.2), vec2<f32>(6.2, -5.2), 1.15);
    let handle = capsule(p, vec2<f32>(-2.0, -7.6), vec2<f32>(2.0, -7.6), 1.05);
    let body = rrect(p, vec4<f32>(-5.2, -3.2, 10.4, 10.6), 1.35);
    return min(lid, min(handle, body));
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = vec4<f32>(in.clip, 0.0, 1.0);
    out.logical = in.logical;
    out.rect = in.rect;
    out.color = in.color;
    out.extra = in.extra;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let mode = in.extra.y;
    let radius = in.extra.x;
    var dist = 0.0;
    if (mode < 0.5) {
        dist = rrect(in.logical, in.rect, radius);
    } else if (mode < 1.5) {
        let c = in.rect.xy + in.rect.zw * 0.5;
        let rad = min(in.rect.z, in.rect.w) * 0.5;
        dist = length(in.logical - c) - rad;
    } else {
        dist = rrect(in.logical, in.rect, radius);
    }

    let aa = max(fwidth(dist), 0.55);
    var alpha = 0.0;
    var rgb = in.color.rgb;
    if (mode > 1.5) {
        alpha = (1.0 - smoothstep(0.0, 18.0, dist)) * in.color.a;
        rgb = vec3<f32>(0.0);
    } else {
        alpha = (1.0 - smoothstep(-aa, aa, dist)) * in.color.a;
        let icon = in.extra.z;
        if (mode < 0.5 && icon > 0.5 && alpha > 0.02) {
            let center = in.rect.xy + in.rect.zw * 0.5;
            let idist = icon_dist(icon, in.logical - center);
            let ia = 1.0 - smoothstep(-0.35, 0.55, idist);
            let icon_col = vec3<f32>(0.890, 0.863, 0.802);
            rgb = mix(rgb, icon_col, ia * in.extra.w);
        }
    }
    if (alpha < 0.004) {
        discard;
    }
    return vec4<f32>(rgb, alpha);
}
