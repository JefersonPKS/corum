#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use corum_assets::model::ModelFile;
use glam::{Vec2, Vec3};
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const DEFAULT_MODEL: &str = "target/verification/model/dfymiss.mod";
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn main() {
    if let Err(error) = run() {
        eprintln!("corum-viewer: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODEL));
    let model = load_model(&path)?;
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let mut application = ViewerApplication::new(path, model);
    event_loop
        .run_app(&mut application)
        .map_err(|error| error.to_string())
}

fn load_model(path: &Path) -> Result<ViewerModel, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "could not read '{}': {error}. Pass a directly exportable MOD path as the first argument",
            path.display()
        )
    })?;
    let model = ModelFile::parse(&bytes).map_err(|error| error.to_string())?;
    ViewerModel::from_model(&model)
}

struct ViewerApplication {
    model_path: PathBuf,
    model: Option<ViewerModel>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
}

impl ViewerApplication {
    fn new(model_path: PathBuf, model: ViewerModel) -> Self {
        Self {
            model_path,
            model: Some(model),
            window: None,
            renderer: None,
        }
    }
}

impl ApplicationHandler for ViewerApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let title = format!(
            "Corum Viewer — {} — arraste: orbitar | roda: zoom | R: reset",
            self.model_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("model.mod")
        );
        let attributes = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(1100.0, 800.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("failed to create window: {error}");
                event_loop.exit();
                return;
            }
        };
        let model = self.model.take().expect("viewer model is initialized once");
        let renderer = match pollster::block_on(Renderer::new(window.clone(), model)) {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!("failed to initialize renderer: {error}");
                event_loop.exit();
                return;
            }
        };

        window.request_redraw();
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window.id() != window_id {
            return;
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => renderer.resize(size),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => renderer.camera.dragging = state == ElementState::Pressed,
            WindowEvent::CursorMoved { position, .. } => {
                renderer.camera.cursor_moved(position);
                window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                renderer.camera.zoom(delta);
                window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                    PhysicalKey::Code(KeyCode::KeyR) => {
                        renderer.camera = OrbitCamera::default();
                        window.request_redraw();
                    }
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                renderer.update_camera();
                if let Err(error) = renderer.render() {
                    eprintln!("render surface error: {error}");
                    renderer.resize(renderer.size);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

#[derive(Debug)]
struct ViewerModel {
    vertices: Vec<Vertex>,
    triangle_count: usize,
}

impl ViewerModel {
    fn from_model(model: &ModelFile) -> Result<Self, String> {
        let mut triangles = Vec::<RawTriangle>::new();
        for mesh in &model.meshes {
            let Some(geometry) = &mesh.geometry else {
                continue;
            };
            for group in &geometry.face_groups {
                for face in &group.faces {
                    let mut positions = [Vec3::ZERO; 3];
                    let mut uvs = [Vec2::ZERO; 3];
                    for corner in 0..3 {
                        let index = usize::from(face[corner]);
                        let position = geometry.positions.get(index).ok_or_else(|| {
                            format!("mesh '{}' contains an invalid position index", mesh.name)
                        })?;
                        let uv = geometry.texture_coordinates.get(index).ok_or_else(|| {
                            format!("mesh '{}' contains an invalid UV index", mesh.name)
                        })?;
                        positions[corner] = Vec3::from_array(*position);
                        uvs[corner] = Vec2::from_array(*uv);
                    }
                    triangles.push(RawTriangle { positions, uvs });
                }
            }
        }

        if triangles.is_empty() {
            return Err(
                "the model has no directly renderable mesh; its seam/skinning layout must be decoded first"
                    .to_owned(),
            );
        }

        let mut minimum = Vec3::splat(f32::INFINITY);
        let mut maximum = Vec3::splat(f32::NEG_INFINITY);
        for triangle in &triangles {
            for position in triangle.positions {
                minimum = minimum.min(position);
                maximum = maximum.max(position);
            }
        }
        let center = (minimum + maximum) * 0.5;
        let largest_extent = (maximum - minimum).max_element().max(0.0001);
        let scale = 2.0 / largest_extent;

        let mut vertices = Vec::with_capacity(triangles.len() * 3);
        for triangle in &triangles {
            let positions = triangle
                .positions
                .map(|position| (position - center) * scale);
            let normal = (positions[1] - positions[0])
                .cross(positions[2] - positions[0])
                .normalize_or(Vec3::Y);
            for (position, uv) in positions.iter().zip(triangle.uvs.iter()) {
                vertices.push(Vertex {
                    position: position.to_array(),
                    normal: normal.to_array(),
                    uv: uv.to_array(),
                });
            }
        }

        Ok(Self {
            vertices,
            triangle_count: triangles.len(),
        })
    }
}

struct RawTriangle {
    positions: [Vec3; 3],
    uvs: [Vec2; 3],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
    light_direction: [f32; 4],
}

struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    depth: DepthTarget,
    camera: OrbitCamera,
}

impl Renderer {
    async fn new(window: Arc<Window>, model: ViewerModel) -> Result<Self, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|error| error.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| error.to_string())?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("corum-viewer device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|error| error.to_string())?;
        let width = size.width.max(1);
        let height = size.height.max(1);
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| "the selected GPU cannot present to this window".to_owned())?;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);

        let camera = OrbitCamera::default();
        let camera_uniform = camera.uniform(width as f32 / height as f32);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera uniform"),
            contents: bytemuck::bytes_of(&camera_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera layout"),
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
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bind group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("corum model shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("model pipeline layout"),
            bind_group_layouts: &[Some(&camera_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("model pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[Some(Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("model vertices"),
            contents: bytemuck::cast_slice(&model.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let depth = DepthTarget::new(&device, width, height);
        eprintln!(
            "loaded {} triangles ({} vertices)",
            model.triangle_count,
            model.vertices.len()
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            pipeline,
            vertex_buffer,
            vertex_count: model.vertices.len() as u32,
            camera_buffer,
            camera_bind_group,
            depth,
            camera,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.size = size;
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth = DepthTarget::new(&self.device, size.width, size.height);
    }

    fn update_camera(&self) {
        if self.config.height == 0 {
            return;
        }
        let uniform = self
            .camera
            .uniform(self.config.width as f32 / self.config.height as f32);
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    fn render(&mut self) -> Result<(), String> {
        if self.size.width == 0 || self.size.height == 0 {
            return Ok(());
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("surface validation error while acquiring the next frame".to_owned());
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("model frame encoder"),
            });
        {
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.025,
                        g: 0.035,
                        b: 0.055,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("model render pass"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.draw(0..self.vertex_count, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        Ok(())
    }
}

struct DepthTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl DepthTarget {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
        }
    }
}

#[derive(Debug)]
struct OrbitCamera {
    yaw: f32,
    pitch: f32,
    distance: f32,
    dragging: bool,
    last_cursor: Option<PhysicalPosition<f64>>,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            yaw: 0.75,
            pitch: 0.35,
            distance: 3.2,
            dragging: false,
            last_cursor: None,
        }
    }
}

impl OrbitCamera {
    fn cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if self.dragging
            && let Some(previous) = self.last_cursor
        {
            self.yaw -= (position.x - previous.x) as f32 * 0.008;
            self.pitch = (self.pitch + (position.y - previous.y) as f32 * 0.008).clamp(-1.45, 1.45);
        }
        self.last_cursor = Some(position);
    }

    fn zoom(&mut self, delta: MouseScrollDelta) {
        let amount = match delta {
            MouseScrollDelta::LineDelta(_, value) => value,
            MouseScrollDelta::PixelDelta(value) => value.y as f32 * 0.02,
        };
        self.distance = (self.distance * (-amount * 0.12).exp()).clamp(1.25, 15.0);
    }

    fn uniform(&self, aspect: f32) -> CameraUniform {
        let horizontal = self.pitch.cos();
        let eye = Vec3::new(
            horizontal * self.yaw.sin(),
            self.pitch.sin(),
            horizontal * self.yaw.cos(),
        ) * self.distance;
        let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::ZERO, Vec3::Y);
        let projection = glam::camera::rh::proj::directx::perspective(
            45_f32.to_radians(),
            aspect.max(0.01),
            0.01,
            100.0,
        );
        CameraUniform {
            view_projection: (projection * view).to_cols_array_2d(),
            light_direction: [-0.35, 0.75, 0.55, 0.0],
        }
    }
}
