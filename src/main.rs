mod document;
mod draw;
mod gpu;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::Vec2;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

use document::Document;
use draw::{Camera, Instance};
use gpu::Gpu;
use ui::{Chrome, Hit, Tool};

const RESIZE_MARGIN: f32 = 6.0;
const TOAST: Duration = Duration::from_millis(1400);

struct DrawGesture {
    samples: Vec<[f32; 2]>,
    tip: Vec2,
    color: [f32; 4],
    width: f32,
}

enum Gesture {
    None,
    Draw(DrawGesture),
    Erase(Vec<(usize, document::Stroke)>),
    Pan { space: bool },
}

struct App {
    path: PathBuf,
    protect_file: bool,
    document: Document,
    camera: Camera,
    gpu: Option<Gpu>,
    tool: Tool,
    color: usize,
    width: usize,
    mods_ctrl: bool,
    mods_shift: bool,
    space: bool,
    gesture: Gesture,
    pointer: Option<Vec2>,
    pointer_world: Vec2,
    hover: Option<Hit>,
    cursor: CursorIcon,
    toast: Option<&'static str>,
    toast_until: Option<Instant>,
    committed: Vec<Instance>,
    committed_gen: u64,
    active: Vec<Instance>,
}

impl App {
    fn new(path: PathBuf) -> Self {
        let mut app = Self {
            path,
            protect_file: false,
            document: Document::default(),
            camera: Camera::default(),
            gpu: None,
            tool: Tool::Pen,
            color: 0,
            width: 0,
            mods_ctrl: false,
            mods_shift: false,
            space: false,
            gesture: Gesture::None,
            pointer: None,
            pointer_world: Vec2::ZERO,
            hover: None,
            cursor: CursorIcon::Crosshair,
            toast: None,
            toast_until: None,
            committed: Vec::new(),
            committed_gen: u64::MAX,
            active: Vec::new(),
        };
        if app.path.exists() {
            match document::load(&app.path) {
                Ok(loaded) => {
                    app.document = loaded.document;
                    app.camera = loaded.camera;
                }
                Err(err) => {
                    eprintln!("ondeck: open failed: {err} ({})", app.path.display());
                    app.protect_file = true;
                    app.toast = Some("open failed");
                    app.toast_until = Some(Instant::now() + Duration::from_millis(2400));
                }
            }
        }
        app
    }

    fn window(&self) -> &Window {
        self.gpu.as_ref().expect("window").window()
    }

    fn view(&self) -> (Vec2, f32, Vec2) {
        let window = self.window();
        let px = window.inner_size();
        let mut scale = window.scale_factor() as f32;
        if !scale.is_finite() || scale <= 0.0 {
            scale = 1.0;
        }
        let logical = Vec2::new(px.width as f32 / scale, px.height as f32 / scale);
        (logical, scale, Vec2::new(px.width as f32, px.height as f32))
    }

    fn redraw(&self) {
        if let Some(gpu) = &self.gpu {
            gpu.window().request_redraw();
        }
    }

    fn set_cursor(&mut self, icon: CursorIcon) {
        if self.cursor != icon {
            self.cursor = icon;
            self.window().set_cursor(icon);
        }
    }

    fn toast(&mut self, text: &'static str) {
        self.toast = Some(text);
        self.toast_until = Some(Instant::now() + TOAST);
        self.redraw();
    }

    fn save(&mut self) -> bool {
        match document::save(&self.path, &self.document, &self.camera) {
            Ok(()) => {
                self.document.dirty = false;
                self.protect_file = false;
                self.toast("saved");
                true
            }
            Err(err) => {
                eprintln!("ondeck: save failed: {err} ({})", self.path.display());
                self.toast("save failed");
                false
            }
        }
    }

    fn finish(&mut self, event_loop: &ActiveEventLoop) {
        self.commit_gesture();
        if self.document.dirty && !self.protect_file {
            if let Err(err) = document::save(&self.path, &self.document, &self.camera) {
                eprintln!("ondeck: save failed: {err} ({})", self.path.display());
            }
        }
        event_loop.exit();
    }

    fn rebuild_committed(&mut self) {
        if self.committed_gen == self.document.revision {
            return;
        }
        self.committed.clear();
        for stroke in &self.document.strokes {
            draw::push_stroke(
                &mut self.committed,
                &stroke.points,
                stroke.width,
                stroke.color,
            );
        }
        self.committed_gen = self.document.revision;
    }

    fn rebuild_active(&mut self) {
        self.active.clear();
        match &self.gesture {
            Gesture::Draw(draw) => {
                let points = draw::render_points(&draw.samples, draw.tip);
                draw::push_stroke(&mut self.active, &points, draw.width, draw.color);
            }
            Gesture::Erase(_) => self.push_eraser_ring(),
            Gesture::None if self.tool == Tool::Eraser && self.hover.is_none() => {
                self.push_eraser_ring();
            }
            _ => {}
        }
    }

    fn push_eraser_ring(&mut self) {
        if self.pointer.is_none() {
            return;
        }
        let radius = 14.0 / self.camera.zoom.max(0.2);
        draw::push_ring(
            &mut self.active,
            self.pointer_world,
            radius,
            [0.77, 0.42, 0.38, 0.9],
        );
    }

    fn update_hover(&mut self) {
        let Some(pointer) = self.pointer else {
            self.hover = None;
            return;
        };
        if !matches!(self.gesture, Gesture::None) {
            return;
        }
        let (view, _, _) = self.view();
        self.hover = ui::hit_test(view, pointer);
        let icon = if let Some(dir) = resize_direction(pointer, view) {
            resize_cursor(dir)
        } else {
            match self.hover {
                Some(Hit::Drag) => CursorIcon::Grab,
                Some(_) => CursorIcon::Pointer,
                None if self.tool == Tool::Eraser => CursorIcon::Cell,
                None => CursorIcon::Crosshair,
            }
        };
        self.set_cursor(icon);
    }

    fn on_pointer(&mut self, logical: Vec2) {
        let previous = self.pointer;
        let previous_world = self.pointer_world;
        self.pointer = Some(logical);
        let (view, _, _) = self.view();
        self.pointer_world = self.camera.screen_to_world(logical, view);
        match &mut self.gesture {
            Gesture::Pan { .. } => {
                if let Some(prev) = previous {
                    self.camera.pan(logical - prev);
                }
            }
            Gesture::Draw(draw) => {
                let min = 0.55 / self.camera.zoom.max(0.05);
                draw.tip = self.pointer_world;
                if draw.samples.is_empty()
                    || Vec2::from(*draw.samples.last().unwrap()).distance(self.pointer_world) >= min
                {
                    draw.samples.push(self.pointer_world.to_array());
                }
            }
            Gesture::Erase(removed) => {
                let radius = 14.0 / self.camera.zoom.max(0.2);
                let samples = sample_segment(previous_world, self.pointer_world, radius * 0.45);
                removed.extend(self.document.extract_hits(&samples, radius));
            }
            Gesture::None => self.update_hover(),
        }
        self.redraw();
    }

    fn on_press(&mut self, button: MouseButton) -> bool {
        let Some(pointer) = self.pointer else {
            return false;
        };
        let (view, _, _) = self.view();
        if button == MouseButton::Left
            && let Some(dir) = resize_direction(pointer, view)
        {
            let _ = self.window().drag_resize_window(dir);
            return false;
        }
        self.update_hover();
        if button == MouseButton::Left
            && let Some(hit) = self.hover
        {
            if hit == Hit::Close {
                return true;
            }
            self.activate(hit);
            return false;
        }
        if button == MouseButton::Middle || (button == MouseButton::Left && self.space) {
            self.gesture = Gesture::Pan {
                space: button == MouseButton::Left,
            };
            self.set_cursor(CursorIcon::Grabbing);
            return false;
        }
        if button != MouseButton::Left {
            return false;
        }
        match self.tool {
            Tool::Pen => {
                self.gesture = Gesture::Draw(DrawGesture {
                    samples: vec![self.pointer_world.to_array()],
                    tip: self.pointer_world,
                    color: draw::PALETTE[self.color],
                    width: draw::WIDTHS[self.width],
                });
            }
            Tool::Eraser => {
                let radius = 14.0 / self.camera.zoom.max(0.2);
                let removed = self
                    .document
                    .extract_hits(&[self.pointer_world], radius);
                self.gesture = Gesture::Erase(removed);
            }
        }
        self.redraw();
        false
    }

    fn on_release(&mut self, button: MouseButton) {
        let end = match &self.gesture {
            Gesture::Pan { space } => {
                (*space && button == MouseButton::Left) || (!*space && button == MouseButton::Middle)
            }
            Gesture::Draw(_) | Gesture::Erase(_) => button == MouseButton::Left,
            Gesture::None => false,
        };
        if end {
            self.commit_gesture();
            self.update_hover();
            self.redraw();
        }
    }

    fn activate(&mut self, hit: Hit) {
        match hit {
            Hit::Drag => {
                let _ = self.window().drag_window();
            }
            Hit::Pen => self.tool = Tool::Pen,
            Hit::Eraser => self.tool = Tool::Eraser,
            Hit::Color(index) => self.color = index as usize,
            Hit::Width(index) => self.width = index as usize,
            Hit::Undo => {
                self.document.undo();
            }
            Hit::Redo => {
                self.document.redo();
            }
            Hit::Clear => {
                self.cancel_gesture();
                self.document.clear();
            }
            Hit::Close => {}
        }
        self.update_hover();
        self.redraw();
    }

    fn commit_gesture(&mut self) {
        match std::mem::replace(&mut self.gesture, Gesture::None) {
            Gesture::Draw(draw) => {
                let points = draw::smooth_stroke(&draw::render_points(&draw.samples, draw.tip));
                self.document.add_stroke(document::Stroke {
                    points,
                    color: draw.color,
                    width: draw.width,
                });
            }
            Gesture::Erase(removed) => self.document.commit_remove(removed),
            Gesture::Pan { .. } | Gesture::None => {}
        }
    }

    fn cancel_gesture(&mut self) {
        match std::mem::replace(&mut self.gesture, Gesture::None) {
            Gesture::Erase(removed) => self.document.restore_removed(&removed),
            _ => {}
        }
    }

    fn key_down(&mut self, code: KeyCode, repeat: bool, event_loop: &ActiveEventLoop) {
        if self.mods_ctrl && code == KeyCode::KeyW {
            self.finish(event_loop);
            return;
        }
        if self.mods_ctrl && code == KeyCode::KeyS && !repeat {
            self.commit_gesture();
            self.save();
            return;
        }
        if !matches!(self.gesture, Gesture::None) {
            if code == KeyCode::Escape && !repeat {
                self.cancel_gesture();
                self.redraw();
            }
            return;
        }
        if repeat && !matches!(code, KeyCode::KeyZ | KeyCode::KeyY) {
            return;
        }
        match code {
            KeyCode::KeyP => self.tool = Tool::Pen,
            KeyCode::KeyE => self.tool = Tool::Eraser,
            KeyCode::KeyZ if self.mods_ctrl && self.mods_shift => {
                self.document.redo();
            }
            KeyCode::KeyZ if self.mods_ctrl => {
                self.document.undo();
            }
            KeyCode::KeyY if self.mods_ctrl => {
                self.document.redo();
            }
            KeyCode::Digit1 => self.color = 0,
            KeyCode::Digit2 => self.color = 1,
            KeyCode::Digit3 => self.color = 2,
            KeyCode::Digit4 => self.color = 3,
            KeyCode::Digit5 => self.color = 4,
            KeyCode::BracketLeft => {
                self.width = self.width.saturating_sub(1);
            }
            KeyCode::BracketRight => {
                self.width = (self.width + 1).min(draw::WIDTHS.len() - 1);
            }
            KeyCode::Digit0 if self.mods_ctrl => {
                self.camera = Camera::default();
            }
            KeyCode::Escape => {}
            _ => return,
        }
        self.update_hover();
        self.redraw();
    }

    fn render(&mut self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let px = gpu.window().inner_size();
        let mut scale = gpu.window().scale_factor() as f32;
        if !scale.is_finite() || scale <= 0.0 {
            scale = 1.0;
        }
        let logical = Vec2::new(px.width as f32 / scale, px.height as f32 / scale);
        let physical = Vec2::new(px.width as f32, px.height as f32);
        self.rebuild_committed();
        self.rebuild_active();
        let show_hint = self.document.strokes.is_empty() && !matches!(self.gesture, Gesture::Draw(_));
        let ui_verts = ui::build(
            logical,
            scale,
            physical,
            &Chrome {
                tool: self.tool,
                color: self.color,
                width: self.width,
                can_undo: self.document.can_undo(),
                can_redo: self.document.can_redo(),
                hover: self.hover,
                toast: self.toast,
                show_hint,
            },
        );
        self.gpu.as_mut().unwrap().render(&gpu::Frame {
            camera: &self.camera,
            scale,
            width: px.width,
            height: px.height,
            committed: &self.committed,
            committed_gen: self.committed_gen,
            active: &self.active,
            ui: &ui_verts,
        });
    }
}

fn sample_segment(a: Vec2, b: Vec2, step: f32) -> Vec<Vec2> {
    let distance = a.distance(b);
    let n = (distance / step.max(0.5)).ceil().max(1.0) as i32;
    (0..=n)
        .map(|i| a.lerp(b, i as f32 / n as f32))
        .collect()
}

fn resize_direction(pointer: Vec2, view: Vec2) -> Option<ResizeDirection> {
    let left = pointer.x <= RESIZE_MARGIN;
    let right = pointer.x >= view.x - RESIZE_MARGIN;
    let top = pointer.y <= RESIZE_MARGIN;
    let bottom = pointer.y >= view.y - RESIZE_MARGIN;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(ResizeDirection::NorthWest),
        (_, true, true, _) => Some(ResizeDirection::NorthEast),
        (true, _, _, true) => Some(ResizeDirection::SouthWest),
        (_, true, _, true) => Some(ResizeDirection::SouthEast),
        (true, _, _, _) => Some(ResizeDirection::West),
        (_, true, _, _) => Some(ResizeDirection::East),
        (_, _, true, _) => Some(ResizeDirection::North),
        (_, _, _, true) => Some(ResizeDirection::South),
        _ => None,
    }
}

fn resize_cursor(dir: ResizeDirection) -> CursorIcon {
    match dir {
        ResizeDirection::East | ResizeDirection::West => CursorIcon::EwResize,
        ResizeDirection::North | ResizeDirection::South => CursorIcon::NsResize,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::NeswResize,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::NwseResize,
    }
}

fn logical_pos(position: PhysicalPosition<f64>, scale: f64) -> Vec2 {
    let logical = position.to_logical::<f32>(scale);
    Vec2::new(logical.x, logical.y)
}

impl winit::application::ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title("ondeck")
            .with_decorations(false)
            .with_resizable(true)
            .with_inner_size(LogicalSize::new(1280.0, 800.0))
            .with_min_inner_size(LogicalSize::new(680.0, 460.0));
        #[cfg(target_os = "linux")]
        {
            use winit::platform::wayland::WindowAttributesExtWayland;
            attrs = attrs.with_name("ondeck", "ondeck");
        }
        let window = Arc::new(event_loop.create_window(attrs).expect("ondeck: failed to open a window"));
        let gpu = pollster::block_on(Gpu::new(event_loop.owned_display_handle(), window));
        gpu.window().set_cursor(self.cursor);
        self.gpu = Some(gpu);
        self.redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.finish(event_loop),
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
                self.redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Focused(true) => self.redraw(),
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.mods_ctrl = state.control_key();
                self.mods_shift = state.shift_key();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.gpu.as_ref().map(|gpu| gpu.window().scale_factor()).unwrap_or(1.0);
                self.on_pointer(logical_pos(position, scale));
            }
            WindowEvent::MouseInput { state, button, .. } => match state {
                ElementState::Pressed => {
                    if self.on_press(button) {
                        self.finish(event_loop);
                    }
                }
                ElementState::Released => self.on_release(button),
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y / 40.0) as f32,
                };
                let steps = steps.clamp(-6.0, 6.0);
                if steps.abs() > 0.01 {
                    let (view, _, _) = self.view();
                    let cursor = self.pointer.unwrap_or(view * 0.5);
                    self.camera.zoom_at(cursor, view, 1.075_f32.powf(steps));
                    if let Some(pointer) = self.pointer {
                        self.pointer_world = self.camera.screen_to_world(pointer, view);
                    }
                    self.redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.physical_key == PhysicalKey::Code(KeyCode::Space) {
                    self.space = event.state == ElementState::Pressed;
                    if !self.space
                        && let Gesture::Pan { space: true } = self.gesture
                    {
                        self.gesture = Gesture::None;
                        self.update_hover();
                        self.redraw();
                    }
                    return;
                }
                if event.state == ElementState::Pressed
                    && let PhysicalKey::Code(code) = event.physical_key
                {
                    self.key_down(code, event.repeat, event_loop);
                }
            }
            WindowEvent::DroppedFile(path) => {
                self.commit_gesture();
                match document::load(&path) {
                    Ok(loaded) => {
                        self.path = path;
                        self.document = loaded.document;
                        self.camera = loaded.camera;
                        self.protect_file = false;
                        self.committed_gen = u64::MAX;
                    }
                    Err(err) => {
                        eprintln!("ondeck: open failed: {err} ({})", path.display());
                        self.toast("open failed");
                    }
                }
                self.redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(until) = self.toast_until {
            if Instant::now() >= until {
                self.toast = None;
                self.toast_until = None;
                self.redraw();
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(until));
                return;
            }
        }
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

fn parse_args() -> PathBuf {
    let mut path = None;
    for arg in std::env::args().skip(1) {
        if arg == "-h" || arg == "--help" {
            println!(
                "\
ondeck — быстрый холст

  ondeck [файл.json]

Без аргумента доска лежит в $XDG_DATA_HOME/ondeck/board.json
(обычно ~/.local/share/ondeck/board.json).
Ctrl+S сохраняет, выход тоже сохраняет, если были правки.

  P перо    E ластик    Space пан    колёсико зум
  Ctrl+Z отмена    Ctrl+Shift+Z повтор    Ctrl+W закрыть
  1–5 цвет    [ ] толщина
"
            );
            std::process::exit(0);
        }
        if path.is_some() {
            eprintln!("ondeck: too many arguments");
            std::process::exit(2);
        }
        path = Some(PathBuf::from(arg));
    }
    path.unwrap_or_else(document::default_path)
}

fn main() {
    env_logger::init();
    let event_loop = EventLoop::new().expect("ondeck: failed to create an event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(parse_args());
    event_loop.run_app(&mut app).expect("ondeck: event loop stopped");
}
