use std::sync::{Arc, Mutex};

use eframe::{
    egui,
    egui_wgpu::{self, wgpu},
};

const INITIAL_CAPACITY: usize = 1024;
const CULL_WORKGROUP_SIZE: u32 = 64;

#[derive(Clone, Copy, Debug, Default)]
pub struct GpuMasonryInputItem {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub target_texture_side: u32,
}

impl GpuMasonryInputItem {
    pub fn from_rect(rect: egui::Rect, target_texture_side: u32) -> Self {
        Self {
            min: [rect.min.x, rect.min.y],
            max: [rect.max.x, rect.max.y],
            target_texture_side,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuMasonryItemRaw {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
    target_texture_side: u32,
    _padding: [u32; 3],
}

impl From<GpuMasonryInputItem> for GpuMasonryItemRaw {
    fn from(item: GpuMasonryInputItem) -> Self {
        Self {
            min_x: item.min[0],
            min_y: item.min[1],
            max_x: item.max[0],
            max_y: item.max[1],
            target_texture_side: item.target_texture_side,
            _padding: [0; 3],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuMasonryMetaRaw {
    item_count: u32,
    _padding0: [u32; 3],
    viewport_min: [f32; 2],
    viewport_max: [f32; 2],
    screen_size: [f32; 2],
    _padding1: [f32; 2],
}

#[derive(Default)]
struct GpuMasonryFrameData {
    viewport_min: [f32; 2],
    viewport_max: [f32; 2],
    items: Vec<GpuMasonryItemRaw>,
}

pub struct GpuMasonryRenderer {
    frame_data: Arc<Mutex<GpuMasonryFrameData>>,
}

impl GpuMasonryRenderer {
    pub fn new(render_state: &egui_wgpu::RenderState) -> Self {
        render_state
            .renderer
            .write()
            .callback_resources
            .insert(GpuMasonryResources::new(
                &render_state.device,
                render_state.target_format,
            ));

        Self {
            frame_data: Arc::new(Mutex::new(GpuMasonryFrameData::default())),
        }
    }

    pub fn enqueue_callback(&self, ui: &mut egui::Ui, items: &[GpuMasonryInputItem]) {
        let callback_rect = ui.clip_rect();

        if let Ok(mut frame_data) = self.frame_data.lock() {
            frame_data.viewport_min = [callback_rect.min.x.max(0.0), callback_rect.min.y.max(0.0)];
            frame_data.viewport_max = [
                callback_rect.max.x.max(frame_data.viewport_min[0]),
                callback_rect.max.y.max(frame_data.viewport_min[1]),
            ];
            frame_data.items.clear();
            frame_data.items.reserve(items.len());
            frame_data
                .items
                .extend(items.iter().copied().map(GpuMasonryItemRaw::from));
        } else {
            return;
        }

        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            callback_rect,
            GpuMasonryCallback {
                frame_data: Arc::clone(&self.frame_data),
            },
        ));
    }
}

struct GpuMasonryCallback {
    frame_data: Arc<Mutex<GpuMasonryFrameData>>,
}

impl egui_wgpu::CallbackTrait for GpuMasonryCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let (items, viewport_min, viewport_max) = match self.frame_data.lock() {
            Ok(frame_data) => (
                frame_data.items.clone(),
                frame_data.viewport_min,
                frame_data.viewport_max,
            ),
            Err(_) => return Vec::new(),
        };

        let Some(resources) = callback_resources.get_mut::<GpuMasonryResources>() else {
            return Vec::new();
        };

        let requested_capacity = items.len().max(1);
        resources.ensure_capacity(device, requested_capacity);

        if !items.is_empty() {
            queue.write_buffer(&resources.item_buffer, 0, bytemuck::cast_slice(&items));
        }

        let meta = GpuMasonryMetaRaw {
            item_count: items.len().min(u32::MAX as usize) as u32,
            _padding0: [0; 3],
            viewport_min,
            viewport_max,
            screen_size: [
                screen_descriptor.size_in_pixels[0] as f32,
                screen_descriptor.size_in_pixels[1] as f32,
            ],
            _padding1: [0.0; 2],
        };
        queue.write_buffer(&resources.meta_buffer, 0, bytemuck::bytes_of(&meta));

        let zero_words = [0u32; 4];
        queue.write_buffer(
            &resources.counter_buffer,
            0,
            bytemuck::cast_slice(&zero_words),
        );
        queue.write_buffer(
            &resources.indirect_buffer,
            0,
            bytemuck::cast_slice(&zero_words),
        );

        let mut compute_pass = egui_encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("masonry_gpu_cull_pass"),
            timestamp_writes: None,
        });
        compute_pass.set_bind_group(0, &resources.compute_bind_group, &[]);

        if !items.is_empty() {
            let dispatch_groups =
                ((items.len() as u32) + CULL_WORKGROUP_SIZE - 1) / CULL_WORKGROUP_SIZE;
            compute_pass.set_pipeline(&resources.cull_pipeline);
            compute_pass.dispatch_workgroups(dispatch_groups, 1, 1);
        }

        compute_pass.set_pipeline(&resources.finalize_pipeline);
        compute_pass.dispatch_workgroups(1, 1, 1);

        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<GpuMasonryResources>() else {
            return;
        };

        render_pass.set_pipeline(&resources.render_pipeline);
        render_pass.set_bind_group(0, &resources.render_bind_group, &[]);
        render_pass.draw_indirect(&resources.indirect_buffer, 0);
    }
}

struct GpuMasonryResources {
    capacity: usize,
    item_buffer: wgpu::Buffer,
    visible_indices_buffer: wgpu::Buffer,
    visible_lod_buffer: wgpu::Buffer,
    indirect_buffer: wgpu::Buffer,
    meta_buffer: wgpu::Buffer,
    counter_buffer: wgpu::Buffer,
    compute_bind_group_layout: wgpu::BindGroupLayout,
    compute_bind_group: wgpu::BindGroup,
    render_bind_group_layout: wgpu::BindGroupLayout,
    render_bind_group: wgpu::BindGroup,
    cull_pipeline: wgpu::ComputePipeline,
    finalize_pipeline: wgpu::ComputePipeline,
    render_pipeline: wgpu::RenderPipeline,
}

impl GpuMasonryResources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let compute_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("masonry_gpu_compute_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let render_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("masonry_gpu_render_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("masonry_gpu_compute_pipeline_layout"),
                bind_group_layouts: &[&compute_bind_group_layout],
                push_constant_ranges: &[],
            });

        let compute_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("masonry_gpu_compute_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/gpu_masonry_compute.wgsl").into(),
            ),
        });

        let cull_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("masonry_gpu_cull_pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_module,
            entry_point: "cull_main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let finalize_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("masonry_gpu_finalize_pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_module,
            entry_point: "finalize_main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("masonry_gpu_render_pipeline_layout"),
                bind_group_layouts: &[&render_bind_group_layout],
                push_constant_ranges: &[],
            });

        let render_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("masonry_gpu_draw_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/gpu_masonry_draw.wgsl").into()),
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("masonry_gpu_draw_pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &render_module,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &render_module,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview: None,
            cache: None,
        });

        let capacity = INITIAL_CAPACITY;
        let item_buffer = create_storage_buffer(
            device,
            "masonry_gpu_item_buffer",
            (capacity * std::mem::size_of::<GpuMasonryItemRaw>()) as u64,
            wgpu::BufferUsages::empty(),
        );
        let visible_indices_buffer = create_storage_buffer(
            device,
            "masonry_gpu_visible_indices_buffer",
            (capacity * std::mem::size_of::<u32>()) as u64,
            wgpu::BufferUsages::empty(),
        );
        let visible_lod_buffer = create_storage_buffer(
            device,
            "masonry_gpu_visible_lod_buffer",
            (capacity * std::mem::size_of::<u32>()) as u64,
            wgpu::BufferUsages::empty(),
        );
        let indirect_buffer = create_storage_buffer(
            device,
            "masonry_gpu_indirect_buffer",
            (4 * std::mem::size_of::<u32>()) as u64,
            wgpu::BufferUsages::INDIRECT,
        );
        let meta_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("masonry_gpu_meta_buffer"),
            size: std::mem::size_of::<GpuMasonryMetaRaw>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let counter_buffer = create_storage_buffer(
            device,
            "masonry_gpu_counter_buffer",
            (4 * std::mem::size_of::<u32>()) as u64,
            wgpu::BufferUsages::empty(),
        );

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("masonry_gpu_compute_bind_group"),
            layout: &compute_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: item_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: visible_indices_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: visible_lod_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: indirect_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: meta_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: counter_buffer.as_entire_binding(),
                },
            ],
        });

        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("masonry_gpu_render_bind_group"),
            layout: &render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: item_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: visible_indices_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: visible_lod_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: meta_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            capacity,
            item_buffer,
            visible_indices_buffer,
            visible_lod_buffer,
            indirect_buffer,
            meta_buffer,
            counter_buffer,
            compute_bind_group_layout,
            compute_bind_group,
            render_bind_group_layout,
            render_bind_group,
            cull_pipeline,
            finalize_pipeline,
            render_pipeline,
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, requested: usize) {
        if requested <= self.capacity {
            return;
        }

        let doubled = self.capacity.saturating_mul(2).max(1);
        let next_pow2 = requested.checked_next_power_of_two().unwrap_or(requested);
        self.capacity = next_pow2.max(doubled);

        self.item_buffer = create_storage_buffer(
            device,
            "masonry_gpu_item_buffer",
            (self.capacity * std::mem::size_of::<GpuMasonryItemRaw>()) as u64,
            wgpu::BufferUsages::empty(),
        );
        self.visible_indices_buffer = create_storage_buffer(
            device,
            "masonry_gpu_visible_indices_buffer",
            (self.capacity * std::mem::size_of::<u32>()) as u64,
            wgpu::BufferUsages::empty(),
        );
        self.visible_lod_buffer = create_storage_buffer(
            device,
            "masonry_gpu_visible_lod_buffer",
            (self.capacity * std::mem::size_of::<u32>()) as u64,
            wgpu::BufferUsages::empty(),
        );

        self.recreate_bind_groups(device);
    }

    fn recreate_bind_groups(&mut self, device: &wgpu::Device) {
        self.compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("masonry_gpu_compute_bind_group"),
            layout: &self.compute_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.item_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.visible_indices_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.visible_lod_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.indirect_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.meta_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.counter_buffer.as_entire_binding(),
                },
            ],
        });

        self.render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("masonry_gpu_render_bind_group"),
            layout: &self.render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.item_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.visible_indices_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.visible_lod_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.meta_buffer.as_entire_binding(),
                },
            ],
        });
    }
}

fn create_storage_buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    additional_usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(4),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | additional_usage,
        mapped_at_creation: false,
    })
}