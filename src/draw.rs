use glam::Vec2;

pub const PALETTE: [[f32; 4]; 5] = [
    hex(0x1c1b19),
    hex(0xc4453c),
    hex(0x3a6fd8),
    hex(0x2f8a62),
    hex(0xd08a2e),
];

pub const WIDTHS: [f32; 3] = [2.35, 4.8, 9.6];

const fn hex(v: u32) -> [f32; 4] {
    [
        ((v >> 16) & 255) as f32 / 255.0,
        ((v >> 8) & 255) as f32 / 255.0,
        (v & 255) as f32 / 255.0,
        1.0,
    ]
}

pub fn linearize(c: [f32; 4]) -> [f32; 4] {
    [
        linear_channel(c[0]),
        linear_channel(c[1]),
        linear_channel(c[2]),
        c[3],
    ]
}

fn linear_channel(s: f32) -> f32 {
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub center: Vec2,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera {
    pub fn screen_to_world(&self, logical: Vec2, view: Vec2) -> Vec2 {
        (logical - view * 0.5) / self.zoom.max(0.0001) + self.center
    }

    pub fn zoom_at(&mut self, logical: Vec2, view: Vec2, factor: f32) {
        let before = self.screen_to_world(logical, view);
        self.zoom = (self.zoom * factor).clamp(0.2, 8.0);
        let after = self.screen_to_world(logical, view);
        self.center += before - after;
    }

    pub fn pan(&mut self, delta_logical: Vec2) {
        self.center -= delta_logical / self.zoom.max(0.0001);
    }
}

/// One GPU instance. `params`: half-width, kind (0 segment, 1 circle, 2 ring).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub color: [f32; 4],
    pub params: [f32; 4],
}

pub fn push_stroke(out: &mut Vec<Instance>, points: &[[f32; 2]], width: f32, color_srgb: [f32; 4]) {
    if points.is_empty() {
        return;
    }
    let color = linearize(color_srgb);
    let half = (width * 0.5).max(0.2);
    if points.len() == 1 {
        push_circle(out, points[0], half, color);
        return;
    }
    let before = out.len();
    for pair in points.windows(2) {
        let a = Vec2::from(pair[0]);
        let b = Vec2::from(pair[1]);
        if a.distance_squared(b) > 1e-6 {
            out.push(Instance {
                a: pair[0],
                b: pair[1],
                color,
                params: [half, 0.0, 0.0, 0.0],
            });
        }
    }
    if out.len() == before {
        push_circle(out, points[0], half, color);
    }
}

fn push_circle(out: &mut Vec<Instance>, center: [f32; 2], half: f32, color: [f32; 4]) {
    out.push(Instance {
        a: center,
        b: center,
        color,
        params: [half, 1.0, 0.0, 0.0],
    });
}

pub fn push_ring(out: &mut Vec<Instance>, center: Vec2, radius: f32, color_srgb: [f32; 4]) {
    out.push(Instance {
        a: center.to_array(),
        b: center.to_array(),
        color: linearize(color_srgb),
        params: [radius, 2.0, 0.0, 0.0],
    });
}

pub fn hits_polyline(points: &[[f32; 2]], width: f32, p: Vec2, radius: f32) -> bool {
    if points.is_empty() {
        return false;
    }
    let reach = radius + width * 0.5;
    let reach2 = reach * reach;
    if points.len() == 1 {
        return Vec2::from(points[0]).distance_squared(p) <= reach2;
    }
    points.windows(2).any(|pair| {
        dist_to_segment(p, Vec2::from(pair[0]), Vec2::from(pair[1])) <= reach
    })
}

fn dist_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-8 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

pub fn smooth_stroke(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if points.len() < 4 {
        return points.to_vec();
    }
    let mut pts: Vec<Vec2> = points.iter().copied().map(Vec2::from).collect();
    for _ in 0..2 {
        let prev = pts.clone();
        for i in 1..pts.len() - 1 {
            pts[i] = prev[i - 1] * 0.25 + prev[i] * 0.5 + prev[i + 1] * 0.25;
        }
    }
    pts.into_iter().map(|p| p.to_array()).collect()
}

pub fn render_points(samples: &[[f32; 2]], tip: Vec2) -> Vec<[f32; 2]> {
    let mut pts = samples.to_vec();
    if pts.last().copied() != Some(tip.to_array()) {
        pts.push(tip.to_array());
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_cursor_still() {
        let mut cam = Camera::default();
        let view = Vec2::new(800.0, 600.0);
        let cursor = Vec2::new(120.0, 80.0);
        let before = cam.screen_to_world(cursor, view);
        cam.zoom_at(cursor, view, 2.4);
        let after = cam.screen_to_world(cursor, view);
        assert!((before - after).length() < 1e-3);
    }

    #[test]
    fn hit_detects_segment_not_only_vertices() {
        let points = [[0.0, 0.0], [100.0, 0.0]];
        assert!(hits_polyline(&points, 4.0, Vec2::new(50.0, 1.0), 2.0));
        assert!(!hits_polyline(&points, 4.0, Vec2::new(50.0, 40.0), 2.0));
    }

    #[test]
    fn smooth_keeps_endpoints() {
        let points = [[0.0, 0.0], [10.0, 8.0], [20.0, 2.0], [30.0, 6.0]];
        let out = smooth_stroke(&points);
        assert_eq!(out.first(), points.first());
        assert_eq!(out.last(), points.last());
    }
}
