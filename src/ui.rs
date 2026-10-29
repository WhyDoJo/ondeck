use glam::Vec2;

use crate::draw::{self, linearize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Eraser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Drag,
    Pen,
    Eraser,
    Color(u8),
    Width(u8),
    Undo,
    Redo,
    Clear,
    Close,
}

#[derive(Clone, Copy)]
struct Target {
    hit: Hit,
    rect: [f32; 4],
}

pub struct Chrome<'a> {
    pub tool: Tool,
    pub color: usize,
    pub width: usize,
    pub can_undo: bool,
    pub can_redo: bool,
    pub hover: Option<Hit>,
    pub toast: Option<&'a str>,
    pub show_hint: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct UiVertex {
    pub clip: [f32; 2],
    pub logical: [f32; 2],
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub extra: [f32; 4],
}

struct Layout {
    targets: Vec<Target>,
    pill: [f32; 4],
}

pub fn hit_test(view: Vec2, p: Vec2) -> Option<Hit> {
    let layout = layout(view);
    for target in layout.targets.iter().rev() {
        if contains(target.rect, p) {
            return Some(target.hit);
        }
    }
    if contains(layout.pill, p) {
        Some(Hit::Drag)
    } else {
        None
    }
}

pub fn build(view: Vec2, scale: f32, px: Vec2, chrome: &Chrome) -> Vec<UiVertex> {
    let layout = layout(view);
    let mut out = Vec::with_capacity(768);
    let to_clip = |p: Vec2| -> [f32; 2] {
        let physical = p * scale;
        [
            physical.x / px.x.max(1.0) * 2.0 - 1.0,
            1.0 - physical.y / px.y.max(1.0) * 2.0,
        ]
    };

    let shadow = [0.0, 0.0, 0.0, 0.16];
    push_quad(
        &mut out,
        expand(layout.pill, 20.0),
        layout.pill,
        shadow,
        [pill_radius(layout.pill), 2.0, 0.0, 0.0],
        &to_clip,
    );
    push_round(
        &mut out,
        layout.pill,
        pill_radius(layout.pill),
        lin(0x1c1b19),
        0.0,
        1.0,
        &to_clip,
    );

    for target in &layout.targets {
        match target.hit {
            Hit::Drag => {}
            Hit::Pen | Hit::Eraser | Hit::Undo | Hit::Redo | Hit::Clear => {
                let selected = match target.hit {
                    Hit::Pen => chrome.tool == Tool::Pen,
                    Hit::Eraser => chrome.tool == Tool::Eraser,
                    _ => false,
                };
                let hovered = chrome.hover == Some(target.hit);
                let enabled = match target.hit {
                    Hit::Undo => chrome.can_undo,
                    Hit::Redo => chrome.can_redo,
                    _ => true,
                };
                let fill = if selected {
                    lin(0x5c5852)
                } else if hovered {
                    lin(0x3a3632)
                } else {
                    lin(0x2a2826)
                };
                let icon = match target.hit {
                    Hit::Pen => 1.0,
                    Hit::Eraser => 2.0,
                    Hit::Undo => 3.0,
                    Hit::Redo => 4.0,
                    Hit::Clear => 5.0,
                    _ => 0.0,
                };
                let strength = if enabled { 1.0 } else { 0.32 };
                push_round(&mut out, target.rect, 11.0, fill, icon, strength, &to_clip);
            }
            Hit::Color(index) => {
                let color = draw::PALETTE[index as usize];
                let selected = chrome.color == index as usize;
                let ring = if selected { 2.6 } else { 1.3 };
                let ring_color = if selected { lin(0xf6f3ee) } else { lin(0x4a4642) };
                push_circle(&mut out, grow(target.rect, ring), ring_color, &to_clip);
                push_circle(&mut out, target.rect, linearize(color), &to_clip);
            }
            Hit::Width(index) => {
                let selected = chrome.width == index as usize;
                let diameter = [5.0, 8.0, 12.0][index as usize];
                let dot = centered_square(target.rect, diameter);
                let color = if selected { lin(0xf4f1ea) } else { lin(0x8f8980) };
                if selected {
                    let ring = centered_square(target.rect, diameter + 7.0);
                    push_circle(&mut out, ring, lin(0xf4f1ea), &to_clip);
                    push_circle(&mut out, inset_rect(ring, 1.6), lin(0x1c1b19), &to_clip);
                }
                push_circle(&mut out, dot, color, &to_clip);
            }
            Hit::Close => {
                let hovered = chrome.hover == Some(Hit::Close);
                let color = if hovered { lin(0xc45048) } else { lin(0x4a4743) };
                let dot = centered_square(target.rect, 13.0);
                push_circle(&mut out, dot, color, &to_clip);
            }
        }
    }

    if let Some(hit) = chrome.hover
        && let Some(label) = label_for(hit)
        && let Some(target) = layout.targets.iter().find(|t| t.hit == hit)
    {
        draw_label(&mut out, target.rect, label, &to_clip);
    }

    if let Some(toast) = chrome.toast {
        draw_toast(&mut out, view, layout.pill, toast, &to_clip);
    }

    draw_text(
        &mut out,
        "ondeck",
        22.0,
        20.0,
        2.0,
        lin(0xc4beb4),
        &to_clip,
    );

    if chrome.show_hint {
        let hint = "space pan    scroll zoom";
        let width = text_width(hint, 2.0);
        draw_text(
            &mut out,
            hint,
            (view.x - width) * 0.5,
            view.y * 0.44,
            2.0,
            lin(0xb7b1a6),
            &to_clip,
        );
    }

    out
}

fn label_for(hit: Hit) -> Option<&'static str> {
    Some(match hit {
        Hit::Pen => "pen",
        Hit::Eraser => "eraser",
        Hit::Undo => "undo",
        Hit::Redo => "redo",
        Hit::Clear => "clear",
        Hit::Close => "close",
        _ => return None,
    })
}

fn draw_label(out: &mut Vec<UiVertex>, button: [f32; 4], text: &str, to_clip: &impl Fn(Vec2) -> [f32; 2]) {
    let scale = 2.0;
    let tw = text_width(text, scale);
    let th = 8.0 * scale;
    let x = button[0] + button[2] * 0.5 - tw * 0.5;
    let y = button[1] - th - 14.0;
    let pad_x = 7.0;
    let pad_y = 4.0;
    let bg = [x - pad_x, y - pad_y, tw + pad_x * 2.0, th + pad_y * 2.0];
    push_round(out, bg, 7.0, lin(0x1c1b19), 0.0, 1.0, to_clip);
    draw_text(out, text, x, y, scale, lin(0xf3f0e8), to_clip);
}

fn draw_toast(
    out: &mut Vec<UiVertex>,
    view: Vec2,
    pill: [f32; 4],
    text: &str,
    to_clip: &impl Fn(Vec2) -> [f32; 2],
) {
    let scale = 2.0;
    let tw = text_width(text, scale);
    let th = 8.0 * scale;
    let x = (view.x - tw) * 0.5;
    let y = pill[1] - th - 22.0;
    let bg = [x - 10.0, y - 6.0, tw + 20.0, th + 12.0];
    push_round(out, bg, 8.0, lin(0x1c1b19), 0.0, 1.0, to_clip);
    draw_text(out, text, x, y, scale, lin(0xf3f0e8), to_clip);
}

fn layout(view: Vec2) -> Layout {
    let natural = 520.0;
    let s = if natural > view.x - 28.0 {
        ((view.x - 28.0) / natural).clamp(0.62, 1.0)
    } else {
        1.0
    };
    let btn = 36.0 * s;
    let gap = 5.0 * s;
    let section = 12.0 * s;
    let pad_x = 10.0 * s;
    let pad_y = 8.0 * s;
    let slot = 24.0 * s;
    let color_d = 16.0 * s;
    let close_slot = 22.0 * s;

    let mut width = pad_x * 2.0;
    width += btn + gap + btn;
    width += section;
    width += slot * 5.0 + gap * 4.0;
    width += section;
    width += slot * 3.0 + gap * 2.0;
    width += section;
    width += btn * 3.0 + gap * 2.0;
    width += section;
    width += close_slot;

    let height = pad_y * 2.0 + btn;
    let origin_x = (view.x - width).max(8.0) * 0.5;
    let origin_y = (view.y - height - 18.0 * s).max(8.0);
    let mut x = origin_x + pad_x;
    let y = origin_y + pad_y;
    let mut targets = Vec::new();

    let place = |targets: &mut Vec<Target>, x: &mut f32, hit: Hit, w: f32| {
        targets.push(Target {
            hit,
            rect: [*x, y, w, btn],
        });
        *x += w + gap;
    };

    place(&mut targets, &mut x, Hit::Pen, btn);
    place(&mut targets, &mut x, Hit::Eraser, btn);
    x += section - gap;

    for i in 0..5 {
        let rect = centered_in([x, y, slot, btn], color_d);
        targets.push(Target {
            hit: Hit::Color(i),
            rect,
        });
        x += slot + gap;
    }
    x += section - gap;

    for i in 0..3 {
        targets.push(Target {
            hit: Hit::Width(i),
            rect: [x, y, slot, btn],
        });
        x += slot + gap;
    }
    x += section - gap;

    place(&mut targets, &mut x, Hit::Undo, btn);
    place(&mut targets, &mut x, Hit::Redo, btn);
    place(&mut targets, &mut x, Hit::Clear, btn);
    x += section - gap;

    targets.push(Target {
        hit: Hit::Close,
        rect: [x, y + (btn - close_slot) * 0.5, close_slot, close_slot],
    });

    Layout {
        targets,
        pill: [origin_x, origin_y, width, height],
    }
}

fn centered_in(slot: [f32; 4], size: f32) -> [f32; 4] {
    [
        slot[0] + (slot[2] - size) * 0.5,
        slot[1] + (slot[3] - size) * 0.5,
        size,
        size,
    ]
}

fn centered_square(rect: [f32; 4], size: f32) -> [f32; 4] {
    [
        rect[0] + (rect[2] - size) * 0.5,
        rect[1] + (rect[3] - size) * 0.5,
        size,
        size,
    ]
}

fn inset_rect(rect: [f32; 4], inset: f32) -> [f32; 4] {
    [
        rect[0] + inset,
        rect[1] + inset,
        (rect[2] - inset * 2.0).max(1.0),
        (rect[3] - inset * 2.0).max(1.0),
    ]
}

fn grow(rect: [f32; 4], pad: f32) -> [f32; 4] {
    [
        rect[0] - pad,
        rect[1] - pad,
        rect[2] + pad * 2.0,
        rect[3] + pad * 2.0,
    ]
}

fn expand(rect: [f32; 4], pad: f32) -> [f32; 4] {
    [
        rect[0] - pad,
        rect[1] - pad,
        rect[2] + pad * 2.0,
        rect[3] + pad * 2.0,
    ]
}

fn pill_radius(pill: [f32; 4]) -> f32 {
    pill[3] * 0.5
}

fn contains(rect: [f32; 4], p: Vec2) -> bool {
    p.x >= rect[0] && p.y >= rect[1] && p.x < rect[0] + rect[2] && p.y < rect[1] + rect[3]
}

fn lin(hex: u32) -> [f32; 4] {
    linearize([
        ((hex >> 16) & 255) as f32 / 255.0,
        ((hex >> 8) & 255) as f32 / 255.0,
        (hex & 255) as f32 / 255.0,
        1.0,
    ])
}

fn push_round(
    out: &mut Vec<UiVertex>,
    rect: [f32; 4],
    radius: f32,
    color: [f32; 4],
    icon: f32,
    strength: f32,
    to_clip: &impl Fn(Vec2) -> [f32; 2],
) {
    push_quad(
        out,
        expand(rect, 1.6),
        rect,
        color,
        [radius, 0.0, icon, strength],
        to_clip,
    );
}

fn push_circle(
    out: &mut Vec<UiVertex>,
    rect: [f32; 4],
    color: [f32; 4],
    to_clip: &impl Fn(Vec2) -> [f32; 2],
) {
    push_quad(out, expand(rect, 1.8), rect, color, [0.0, 1.0, 0.0, 1.0], to_clip);
}

fn push_quad(
    out: &mut Vec<UiVertex>,
    bounds: [f32; 4],
    rect: [f32; 4],
    color: [f32; 4],
    extra: [f32; 4],
    to_clip: &impl Fn(Vec2) -> [f32; 2],
) {
    let (x, y, w, h) = (bounds[0], bounds[1], bounds[2], bounds[3]);
    let corners = [
        Vec2::new(x, y),
        Vec2::new(x + w, y),
        Vec2::new(x + w, y + h),
        Vec2::new(x, y + h),
    ];
    for index in [0, 1, 2, 0, 2, 3] {
        let p = corners[index];
        out.push(UiVertex {
            clip: to_clip(p),
            logical: [p.x, p.y],
            rect,
            color,
            extra,
        });
    }
}

fn draw_text(
    out: &mut Vec<UiVertex>,
    text: &str,
    x: f32,
    y: f32,
    scale: f32,
    color: [f32; 4],
    to_clip: &impl Fn(Vec2) -> [f32; 2],
) {
    let mut cursor = x;
    for ch in text.chars() {
        if let Some(rows) = glyph(ch) {
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if bits & (1 << (4 - col)) != 0 {
                        let rect = [
                            cursor + col as f32 * scale,
                            y + row as f32 * scale,
                            scale,
                            scale,
                        ];
                        push_round(out, rect, 0.0, color, 0.0, 1.0, to_clip);
                    }
                }
            }
        }
        cursor += 6.0 * scale;
    }
}

fn text_width(text: &str, scale: f32) -> f32 {
    text.chars().count() as f32 * 6.0 * scale
}

fn glyph(ch: char) -> Option<[u8; 7]> {
    Some(match ch {
        'a' => [0x00, 0x0E, 0x01, 0x0F, 0x11, 0x0F, 0x00],
        'b' => [0x10, 0x10, 0x16, 0x19, 0x11, 0x1E, 0x00],
        'c' => [0x00, 0x0E, 0x10, 0x10, 0x10, 0x0E, 0x00],
        'd' => [0x01, 0x01, 0x0D, 0x13, 0x11, 0x0F, 0x00],
        'e' => [0x00, 0x0E, 0x11, 0x1E, 0x10, 0x0E, 0x00],
        'f' => [0x06, 0x08, 0x1C, 0x08, 0x08, 0x08, 0x00],
        'g' => [0x00, 0x0F, 0x11, 0x0F, 0x01, 0x0E, 0x00],
        'h' => [0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x00],
        'i' => [0x04, 0x00, 0x0C, 0x04, 0x04, 0x0E, 0x00],
        'j' => [0x02, 0x00, 0x06, 0x02, 0x12, 0x0C, 0x00],
        'k' => [0x10, 0x12, 0x14, 0x18, 0x14, 0x12, 0x00],
        'l' => [0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E, 0x00],
        'm' => [0x00, 0x1A, 0x15, 0x15, 0x15, 0x15, 0x00],
        'n' => [0x00, 0x16, 0x19, 0x11, 0x11, 0x11, 0x00],
        'o' => [0x00, 0x0E, 0x11, 0x11, 0x11, 0x0E, 0x00],
        'p' => [0x00, 0x1E, 0x11, 0x1E, 0x10, 0x10, 0x00],
        'q' => [0x00, 0x0F, 0x11, 0x0F, 0x01, 0x01, 0x00],
        'r' => [0x00, 0x16, 0x19, 0x10, 0x10, 0x10, 0x00],
        's' => [0x00, 0x0F, 0x10, 0x0E, 0x01, 0x1E, 0x00],
        't' => [0x04, 0x0E, 0x04, 0x04, 0x04, 0x03, 0x00],
        'u' => [0x00, 0x11, 0x11, 0x11, 0x13, 0x0D, 0x00],
        'v' => [0x00, 0x11, 0x11, 0x11, 0x0A, 0x04, 0x00],
        'w' => [0x00, 0x11, 0x11, 0x15, 0x15, 0x0A, 0x00],
        'x' => [0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x00],
        'y' => [0x00, 0x11, 0x11, 0x0F, 0x01, 0x0E, 0x00],
        'z' => [0x00, 0x1F, 0x02, 0x04, 0x08, 0x1F, 0x00],
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_covers_chrome() {
        let text = "pen eraser undo redo clear close saved save failed space pan scroll zoom ondeck";
        for ch in text.chars() {
            assert!(glyph(ch).is_some(), "missing glyph {ch}");
        }
    }
}
