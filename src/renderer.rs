//! GPU-accelerated rendering module using wgpu
//! 
//! Handles all rendering operations including:
//! - Image texture management
//! - Shader-based rendering with zoom and pan
//! - UI overlay rendering (control buttons)
//! - Smooth animation interpolation

use bytemuck::{Pod, Zeroable};
use log::info;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::image_loader::ImageFrame;

/// Vertex data for a quad
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
}

impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
        0 => Float32x2,
        1 => Float32x2
    ];
    
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Quad vertices (two triangles)
const QUAD_VERTICES: &[Vertex] = &[
    Vertex { position: [-1.0, -1.0], tex_coords: [0.0, 1.0] },
    Vertex { position: [1.0, -1.0], tex_coords: [1.0, 1.0] },
    Vertex { position: [1.0, 1.0], tex_coords: [1.0, 0.0] },
    Vertex { position: [-1.0, -1.0], tex_coords: [0.0, 1.0] },
    Vertex { position: [1.0, 1.0], tex_coords: [1.0, 0.0] },
    Vertex { position: [-1.0, 1.0], tex_coords: [0.0, 0.0] },
];

/// Uniform data passed to shaders
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Uniforms {
    /// Transform matrix (scale, rotation, translation)
    transform: [[f32; 4]; 4],
    /// Image opacity (for fade animations)
    opacity: f32,
    /// Rotation in radians
    rotation: f32,
    /// Padding for alignment
    _padding: [f32; 2],
}

impl Default for Uniforms {
    fn default() -> Self {
        Self {
            transform: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            opacity: 1.0,
            rotation: 0.0,
            _padding: [0.0; 2],
        }
    }
}

/// Button uniform for UI rendering
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct ButtonUniform {
    /// Button position (x, y) in normalized coordinates
    position: [f32; 2],
    /// Button size (width, height) in normalized coordinates
    size: [f32; 2],
    /// Button color (RGBA)
    color: [f32; 4],
    /// Hover state (0.0 or 1.0)
    hover: f32,
    /// Button type (0=minimize, 1=maximize, 2=close)
    button_type: f32,
    /// Padding
    _padding: [f32; 2],
}

/// The main renderer
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    
    // Image rendering
    image_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    
    // Current texture
    texture: Option<wgpu::Texture>,
    texture_view: Option<wgpu::TextureView>,
    texture_bind_group: Option<wgpu::BindGroup>,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    
    // UI rendering
    ui_pipeline: wgpu::RenderPipeline,
    button_buffer: wgpu::Buffer,
    button_bind_group: wgpu::BindGroup,
    
    // Current state
    uniforms: Uniforms,
}

impl Renderer {
    /// Create a new renderer
    pub async fn new(window: Arc<Window>) -> Result<Self, Box<dyn std::error::Error>> {
        let size = window.inner_size();
        
        // Create wgpu instance
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        
        // Create surface
        let surface = instance.create_surface(window.clone())?;
        
        // Request adapter
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or("Failed to find suitable GPU adapter")?;
        
        info!("Using GPU: {:?}", adapter.get_info().name);
        
        // Create device and queue
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Main Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await?;
        
        // Configure surface
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);
        
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        
        // Create vertex buffer
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(QUAD_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        
        // Create uniform buffer
        let uniforms = Uniforms::default();
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        
        // Create uniform bind group layout
        let uniform_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Uniform Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uniform Bind Group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        
        // Create texture bind group layout
        let texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Texture Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        
        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Image Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        
        // Create shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Image Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/image.wgsl").into()),
        });
        
        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Image Pipeline Layout"),
            bind_group_layouts: &[&uniform_bind_group_layout, &texture_bind_group_layout],
            push_constant_ranges: &[],
        });
        
        // Create render pipeline
        let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Image Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        
        // Create UI shader and pipeline
        let ui_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UI Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ui.wgsl").into()),
        });
        
        let button_uniform = ButtonUniform {
            position: [0.0, 0.0],
            size: [0.1, 0.05],
            color: [0.2, 0.2, 0.2, 0.8],
            hover: 0.0,
            button_type: 0.0,
            _padding: [0.0; 2],
        };
        
        let button_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Button Buffer"),
            contents: bytemuck::cast_slice(&[button_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        
        let button_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Button Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        
        let button_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Button Bind Group"),
            layout: &button_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: button_buffer.as_entire_binding(),
            }],
        });
        
        let ui_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("UI Pipeline Layout"),
            bind_group_layouts: &[&button_bind_group_layout],
            push_constant_ranges: &[],
        });
        
        let ui_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("UI Pipeline"),
            layout: Some(&ui_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &ui_shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &ui_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        
        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            image_pipeline,
            vertex_buffer,
            uniform_buffer,
            uniform_bind_group,
            texture: None,
            texture_view: None,
            texture_bind_group: None,
            texture_bind_group_layout,
            sampler,
            ui_pipeline,
            button_buffer,
            button_bind_group,
            uniforms,
        })
    }
    
    /// Resize the renderer
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }
    
    /// Upload an image frame to the GPU
    pub fn upload_image(&mut self, frame: &ImageFrame) {
        let texture_size = wgpu::Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        };
        
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Image Texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * frame.width),
                rows_per_image: Some(frame.height),
            },
            texture_size,
        );
        
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        let texture_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Texture Bind Group"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        
        self.texture = Some(texture);
        self.texture_view = Some(texture_view);
        self.texture_bind_group = Some(texture_bind_group);
    }
    
    /// Update the transform uniforms
    pub fn update_transform(
        &mut self,
        scale: f32,
        offset_x: f32,
        offset_y: f32,
        rotation_degrees: u32,
        opacity: f32,
        image_aspect: f32,
    ) {
        let rotation_rad = (rotation_degrees as f32).to_radians();
        let cos_r = rotation_rad.cos();
        let sin_r = rotation_rad.sin();
        
        // Calculate aspect ratio correction
        let window_aspect = self.size.width as f32 / self.size.height as f32;
        let aspect_correction = if rotation_degrees == 90 || rotation_degrees == 270 {
            // Swap aspect for 90/270 degree rotations
            (1.0 / image_aspect) / window_aspect
        } else {
            image_aspect / window_aspect
        };
        
        // Build transform matrix: Scale -> Rotate -> Translate
        let sx = scale * aspect_correction.min(1.0);
        let sy = scale * (1.0 / aspect_correction).min(1.0);
        
        // Rotation matrix
        let rot = [
            [cos_r, -sin_r, 0.0, 0.0],
            [sin_r, cos_r, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        
        // Scale matrix
        let scl = [
            [sx, 0.0, 0.0, 0.0],
            [0.0, sy, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        
        // Translation
        let trans = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [offset_x * 2.0, offset_y * 2.0, 0.0, 1.0],
        ];
        
        // Combine: T * R * S
        let transform = mat4_multiply(&trans, &mat4_multiply(&rot, &scl));
        
        self.uniforms = Uniforms {
            transform,
            opacity,
            rotation: rotation_rad,
            _padding: [0.0; 2],
        };
        
        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[self.uniforms]));
    }
    
    /// Render the current frame
    pub fn render(&mut self, show_controls: bool, button_states: &[bool; 3]) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });
        
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            
            // Render image if texture is loaded
            if let Some(ref texture_bind_group) = self.texture_bind_group {
                render_pass.set_pipeline(&self.image_pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_bind_group(1, texture_bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.draw(0..6, 0..1);
            }
        }
        
        // Render UI buttons if controls are visible
        if show_controls {
            self.render_buttons(&mut encoder, &view, button_states);
        }
        
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        
        Ok(())
    }
    
    /// Render control buttons
    fn render_buttons(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        button_states: &[bool; 3],
    ) {
        // Buttons: [minimize, maximize, close]
        let button_width = 46.0 / self.size.width as f32;
        let button_height = 32.0 / self.size.height as f32;
        let padding = 2.0 / self.size.width as f32;
        
        let buttons = [
            // Minimize button
            ButtonUniform {
                position: [1.0 - (button_width + padding) * 3.0, 1.0 - button_height - padding],
                size: [button_width, button_height],
                color: if button_states[0] { [0.4, 0.4, 0.4, 0.9] } else { [0.2, 0.2, 0.2, 0.85] },
                hover: if button_states[0] { 1.0 } else { 0.0 },
                button_type: 0.0,
                _padding: [0.0; 2],
            },
            // Maximize button
            ButtonUniform {
                position: [1.0 - (button_width + padding) * 2.0, 1.0 - button_height - padding],
                size: [button_width, button_height],
                color: if button_states[1] { [0.4, 0.4, 0.4, 0.9] } else { [0.2, 0.2, 0.2, 0.85] },
                hover: if button_states[1] { 1.0 } else { 0.0 },
                button_type: 1.0,
                _padding: [0.0; 2],
            },
            // Close button
            ButtonUniform {
                position: [1.0 - button_width - padding, 1.0 - button_height - padding],
                size: [button_width, button_height],
                color: if button_states[2] { [0.9, 0.2, 0.2, 0.9] } else { [0.7, 0.1, 0.1, 0.85] },
                hover: if button_states[2] { 1.0 } else { 0.0 },
                button_type: 2.0,
                _padding: [0.0; 2],
            },
        ];
        
        for button in &buttons {
            self.queue.write_buffer(&self.button_buffer, 0, bytemuck::cast_slice(&[*button]));
            
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("UI Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            
            render_pass.set_pipeline(&self.ui_pipeline);
            render_pass.set_bind_group(0, &self.button_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.draw(0..6, 0..1);
        }
    }
    
    /// Get current window size
    pub fn size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.size
    }
}

/// Matrix multiplication for 4x4 matrices
fn mat4_multiply(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut result = [[0.0f32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            for k in 0..4 {
                result[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    result
}
