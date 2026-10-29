use std::sync::Arc;

use winit::window::Window;

use crate::draw::{Camera, Instance};
use crate::ui::UiVertex;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    center: [f32; 2],
    zoom: f32,
    scale: f32,
    viewport_px: [f32; 2],
    _pad: [f32; 2],
}

const QUAD: [[f32; 2]; 6] = [
    [-1.0, -1.0],
    [1.0, -1.0],
    [1.0, 1.0],
    [-1.0, -1.0],
    [1.0, 1.0],
    [-1.0, 1.0],
];

pub struct Gpu {
    window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    view_format: wgpu::TextureFormat,
    camera_buffer: wgpu::Buffer,
    camera_bind: wgpu::BindGroup,
    stroke_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
    ui_pipeline: wgpu::RenderPipeline,
    quad: wgpu::Buffer,
    committed: DynBuffer,
    active: DynBuffer,
    ui: DynBuffer,
    committed_gen: u64,
    committed_count: u32,
    timeouts: u8,
}

pub struct Frame<'a> {
    pub camera: &'a Camera,
    pub scale: f32,
    pub width: u32,
    pub height: u32,
    pub committed: &'a [Instance],
    pub committed_gen: u64,
    pub active: &'a [Instance],
    pub ui: &'a [UiVertex],
}

impl Gpu {
    pub async fn new(display: winit::event_loop::OwnedDisplayHandle, window: Arc<Window>) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(display),
        ));
        let surface = instance
            .create_surface(window.clone())
            .expect("ondeck: failed to create a Wayland surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("ondeck: no GPU adapter. Vulkan is required");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .expect("ondeck: failed to open the GPU");

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|format| {
                matches!(
                    format,
                    wgpu::TextureFormat::Bgra8UnormSrgb
                        | wgpu::TextureFormat::Rgba8UnormSrgb
                        | wgpu::TextureFormat::Bgra8Unorm
                        | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .unwrap_or(caps.formats[0]);
        let view_format = format.add_srgb_suffix();
        let present_mode = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Fifo]
            .into_iter()
            .find(|mode| caps.present_modes.contains(mode))
            .unwrap_or(wgpu::PresentMode::AutoVsync);
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            wgpu::CompositeAlphaMode::Auto
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: vec![view_format],
            alpha_mode,
            width: size.width.max(1),
            height: size.height.max(1),
            desired_maximum_frame_latency: 1,
            present_mode,
        };
        surface.configure(&device, &config);

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let stroke_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stroke"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/stroke.wgsl").into()),
        });
        let grid_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/grid.wgsl").into()),
        });
        let ui_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ui.wgsl").into()),
        });

        let stroke_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("stroke"),
            bind_group_layouts: &[Some(&camera_layout)],
            immediate_size: 0,
        });
        let empty_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });

        let stroke_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("stroke"),
            layout: Some(&stroke_layout),
            vertex: wgpu::VertexState {
                module: &stroke_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Instance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            1 => Float32x2,
                            2 => Float32x2,
                            3 => Float32x4,
                            4 => Float32x4
                        ],
                    }),
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &stroke_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(color_target(view_format))],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let grid_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid"),
            layout: Some(&stroke_layout),
            vertex: wgpu::VertexState {
                module: &grid_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &grid_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(color_target(view_format))],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let ui_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui"),
            layout: Some(&empty_layout),
            vertex: wgpu::VertexState {
                module: &ui_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<UiVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4,
                        3 => Float32x4,
                        4 => Float32x4
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &ui_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(color_target(view_format))],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let quad = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad"),
            size: std::mem::size_of_val(&QUAD) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&quad, 0, bytemuck::bytes_of(&QUAD));

        Self {
            window,
            instance,
            surface,
            device,
            queue,
            config,
            view_format,
            camera_buffer,
            camera_bind,
            stroke_pipeline,
            grid_pipeline,
            ui_pipeline,
            quad,
            committed: DynBuffer::new(wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST),
            active: DynBuffer::new(wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST),
            ui: DynBuffer::new(wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST),
            committed_gen: u64::MAX,
            committed_count: 0,
            timeouts: 0,
        }
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn render(&mut self, frame: &Frame) {
        if frame.width == 0 || frame.height == 0 {
            return;
        }
        if self.config.width != frame.width || self.config.height != frame.height {
            self.resize(frame.width, frame.height);
        }

        let uniform = CameraUniform {
            center: frame.camera.center.to_array(),
            zoom: frame.camera.zoom,
            scale: frame.scale,
            viewport_px: [frame.width as f32, frame.height as f32],
            _pad: [0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));

        if self.committed_gen != frame.committed_gen {
            self.committed
                .write(&self.device, &self.queue, bytemuck::cast_slice(frame.committed));
            self.committed_count = frame.committed.len() as u32;
            self.committed_gen = frame.committed_gen;
        }
        self.active
            .write(&self.device, &self.queue, bytemuck::cast_slice(frame.active));
        self.ui
            .write(&self.device, &self.queue, bytemuck::cast_slice(frame.ui));
        let active_count = frame.active.len() as u32;
        let ui_count = frame.ui.len() as u32;

        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                drop(texture);
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                if self.timeouts < 6 {
                    self.timeouts += 1;
                    self.window.request_redraw();
                }
                return;
            }
            wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                if let Ok(surface) = self.instance.create_surface(self.window.clone()) {
                    self.surface = surface;
                    self.surface.configure(&self.device, &self.config);
                }
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => return,
        };
        self.timeouts = 0;

        let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.89,
                            g: 0.86,
                            b: 0.80,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.camera_bind, &[]);
            pass.set_pipeline(&self.grid_pipeline);
            pass.draw(0..3, 0..1);

            pass.set_pipeline(&self.stroke_pipeline);
            pass.set_vertex_buffer(0, self.quad.slice(..));
            if self.committed_count > 0 {
                pass.set_vertex_buffer(1, self.committed.buffer.as_ref().unwrap().slice(..));
                pass.draw(0..6, 0..self.committed_count);
            }
            if active_count > 0 {
                pass.set_vertex_buffer(1, self.active.buffer.as_ref().unwrap().slice(..));
                pass.draw(0..6, 0..active_count);
            }

            if ui_count > 0 {
                pass.set_pipeline(&self.ui_pipeline);
                pass.set_vertex_buffer(0, self.ui.buffer.as_ref().unwrap().slice(..));
                pass.draw(0..ui_count, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(surface_texture);
    }
}

fn color_target(format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    }
}

struct DynBuffer {
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    usage: wgpu::BufferUsages,
}

impl DynBuffer {
    fn new(usage: wgpu::BufferUsages) -> Self {
        Self {
            buffer: None,
            capacity: 0,
            usage,
        }
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let size = data.len() as u64;
        if size > self.capacity {
            let mut capacity = self.capacity.max(4096);
            while capacity < size {
                capacity *= 2;
            }
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dynamic"),
                size: capacity,
                usage: self.usage,
                mapped_at_creation: false,
            }));
            self.capacity = capacity;
        }
        queue.write_buffer(self.buffer.as_ref().unwrap(), 0, data);
    }
}
