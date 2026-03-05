use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use eframe::{
    egui,
    egui_wgpu::{self, wgpu},
};

const INITIAL_CAPACITY: usize = 1024;
const CULL_WORKGROUP_SIZE: u32 = 64;
const ATLAS_MAX_SIZE: u32 = 4096;
const ATLAS_PADDING: u32 = 1;

#[derive(Clone, Copy, Debug, Default)]
pub struct GpuMasonryInputItem {
    pub index: usize,
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl GpuMasonryInputItem {
    pub fn from_rect(index: usize, rect: egui::Rect, _target_texture_side: u32) -> Self {
        Self {
            index,
            min: [rect.min.x, rect.min.y],
            max: [rect.max.x, rect.max.y],
        }
    }
}

#[derive(Clone)]
struct PendingTextureUpload {
    index: usize,
    width: u32,
    height: u32,
    lod_side: u32,
    pixels: Vec<u8>,
}

#[derive(Clone, Copy)]
struct AtlasEntry {
    uv_min: [f32; 2],
    uv_max: [f32; 2],
    lod_side: u32,
}

#[derive(Default)]
struct GpuMasonryFrameData {
    viewport_min: [f32; 2],
    viewport_max: [f32; 2],
    items: Vec<GpuMasonryInputItem>,
}

#[derive(Default)]
struct GpuMasonrySharedState {
    frame_data: GpuMasonryFrameData,
    pending_uploads: VecDeque<PendingTextureUpload>,
    atlas_entries: HashMap<usize, AtlasEntry>,
    atlas_reset_requested: bool,
}

pub struct GpuMasonryRenderer {
    shared: Arc<Mutex<GpuMasonrySharedState>>,
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
            shared: Arc::new(Mutex::new(GpuMasonrySharedState::default())),
        }
    }

    pub fn enqueue_texture_upload(
        &self,
        index: usize,
        width: u32,
        height: u32,
        lod_side: u32,
        pixels: Vec<u8>,
    ) {
        if width == 0 || height == 0 || pixels.is_empty() {
            return;
        }

        let Ok(mut shared) = self.shared.lock() else {
            return;
        };

        if let Some(existing) = shared.pending_uploads.iter_mut().find(|u| u.index == index) {
            *existing = PendingTextureUpload {
                index,
                width,
                height,
                lod_side,
                pixels,
            };
            return;
        }

        shared.pending_uploads.push_back(PendingTextureUpload {
            index,
            width,
            height,
            lod_side,
            pixels,
        });
    }

    pub fn remove_texture(&self, index: usize) {
        let Ok(mut shared) = self.shared.lock() else {
            return;
        };

        shared.atlas_entries.remove(&index);
        shared.pending_uploads.retain(|u| u.index != index);
    }

    pub fn clear_textures(&self) {
        let Ok(mut shared) = self.shared.lock() else {
            return;
        };

        shared.pending_uploads.clear();
        shared.atlas_entries.clear();
        shared.atlas_reset_requested = true;
    }

    pub fn can_render_index(&self, index: usize) -> bool {
        let Ok(shared) = self.shared.lock() else {
            return false;
        };

        shared.atlas_entries.contains_key(&index)
    }

    pub fn needs_lod_upgrade(&self, index: usize, target_side: u32) -> bool {
        let Ok(shared) = self.shared.lock() else {
            return false;
        };

        let Some(entry) = shared.atlas_entries.get(&index) else {
            return false;
        };

        lod_rank_for_side(entry.lod_side) < lod_rank_for_side(target_side)
    }

    pub fn enqueue_callback(&self, ui: &mut egui::Ui, items: &[GpuMasonryInputItem]) {
        let callback_rect = ui.clip_rect();

        if let Ok(mut shared) = self.shared.lock() {
            shared.frame_data.viewport_min = [callback_rect.min.x.max(0.0), callback_rect.min.y.max(0.0)];
            shared.frame_data.viewport_max = [
                callback_rect.max.x.max(shared.frame_data.viewport_min[0]),
                callback_rect.max.y.max(shared.frame_data.viewport_min[1]),
            ];
            shared.frame_data.items.clear();
            shared.frame_data.items.extend_from_slice(items);
        } else {
            return;
        }

        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            callback_rect,
            GpuMasonryCallback {
                shared: Arc::clone(&self.shared),
            },
        ));
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuMasonryItemRaw {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
    uv_min_x: f32,
    uv_min_y: f32,
    uv_max_x: f32,
    uv_max_y: f32,
    lod_side: u32,
    ready: u32,
    _padding: [u32; 2],
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

struct GpuMasonryCallback {
    shared: Arc<Mutex<GpuMasonrySharedState>>,
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
        let Some(resources) = callback_resources.get_mut::<GpuMasonryResources>() else {
            return Vec::new();
        };

        let (frame_items, viewport_min, viewport_max, uploads, reset_requested) = {
            let Ok(mut shared) = self.shared.lock() else {
                return Vec::new();
            };

            let frame_items = shared.frame_data.items.clone();
            let viewport_min = shared.frame_data.viewport_min;
            let viewport_max = shared.frame_data.viewport_max;
            let uploads: Vec<PendingTextureUpload> = shared.pending_uploads.drain(..).collect();
            let reset_requested = std::mem::take(&mut shared.atlas_reset_requested);

            (
                frame_items,
                viewport_min,
                viewport_max,
                uploads,
                reset_requested,
            )
        };

        if reset_requested {
            resources.reset_atlas(device);
            if let Ok(mut shared) = self.shared.lock() {
                shared.atlas_entries.clear();
            }
        }

        if !uploads.is_empty() {
            let Ok(mut shared) = self.shared.lock() else {
                return Vec::new();
            };
            for upload in uploads {
                resources.upload_to_atlas(device, queue, &mut shared.atlas_entries, upload);
            }
        }

        let raw_items = {
            let Ok(shared) = self.shared.lock() else {
                return Vec::new();
            };

            let mut raw_items = Vec::with_capacity(frame_items.len());
            for item in frame_items {
                if let Some(entry) = shared.atlas_entries.get(&item.index) {
                    raw_items.push(GpuMasonryItemRaw {
                        min_x: item.min[0],
                        min_y: item.min[1],
                        max_x: item.max[0],
                        max_y: item.max[1],
                        uv_min_x: entry.uv_min[0],
                        uv_min_y: entry.uv_min[1],
                        uv_max_x: entry.uv_max[0],
                        uv_max_y: entry.uv_max[1],
                        lod_side: entry.lod_side,
                        ready: 1,
                        _padding: [0; 2],
                    });
                } else {
                    raw_items.push(GpuMasonryItemRaw {
                        min_x: item.min[0],
                        min_y: item.min[1],
                        max_x: item.max[0],
                        max_y: item.max[1],
                        uv_min_x: 0.0,
                        uv_min_y: 0.0,
                        uv_max_x: 0.0,
                        uv_max_y: 0.0,
                        lod_side: 0,
                        ready: 0,
                        _padding: [0; 2],
                    });
                }
            }

            raw_items
        };

        resources.ensure_capacity(device, raw_items.len().max(1));

        if !raw_items.is_empty() {
            queue.write_buffer(&resources.item_buffer, 0, bytemuck::cast_slice(&raw_items));
        }

        let meta = GpuMasonryMetaRaw {
            item_count: raw_items.len().min(u32::MAX as usize) as u32,
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

        if !raw_items.is_empty() {
            let dispatch_groups = ((raw_items.len() as u32) + CULL_WORKGROUP_SIZE - 1) / CULL_WORKGROUP_SIZE;
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

#[derive(Clone, Copy)]
struct AtlasAllocator {
    size: u32,
    next_x: u32,
    next_y: u32,
    row_height: u32,
}

impl AtlasAllocator {
    fn new(size: u32) -> Self {
        Self {
            size,
            next_x: 0,
            next_y: 0,
            row_height: 0,
        }
    }

    fn reset(&mut self) {
        self.next_x = 0;
        self.next_y = 0;
        self.row_height = 0;
    }

    fn allocate(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if width > self.size || height > self.size {
            return None;
        }

        if self.next_x + width > self.size {
            self.next_x = 0;
            self.next_y = self.next_y.saturating_add(self.row_height);
            self.row_height = 0;
        }

        if self.next_y + height > self.size {
            return None;
        }

        let origin = (self.next_x, self.next_y);
        self.next_x = self.next_x.saturating_add(width);
        self.row_height = self.row_height.max(height);

        Some(origin)
    }
}

struct GpuMasonryResources {
    capacity: usize,
    item_buffer: wgpu::Buffer,
    visible_indices_buffer: wgpu::Buffer,
    indirect_buffer: wgpu::Buffer,
    meta_buffer: wgpu::Buffer,
    counter_buffer: wgpu::Buffer,
    compute_bind_group_layout: wgpu::BindGroupLayout,
    compute_bind_group: wgpu::BindGroup,
    render_bind_group_layout: wgpu::BindGroupLayout,
    render_bind_group: wgpu::BindGroup,
    atlas_size: u32,
    atlas_texture: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    atlas_sampler: wgpu::Sampler,
    atlas_allocator: AtlasAllocator,
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
                        visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::VERTEX,
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
                        visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
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
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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

        let atlas_size = device.limits().max_texture_dimension_2d.min(ATLAS_MAX_SIZE).max(1024);
        let atlas_texture = create_atlas_texture(device, atlas_size);
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("masonry_gpu_atlas_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

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
                    resource: indirect_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: meta_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
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
                    resource: meta_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
        });

        let mut resources = Self {
            capacity,
            item_buffer,
            visible_indices_buffer,
            indirect_buffer,
            meta_buffer,
            counter_buffer,
            compute_bind_group_layout,
            compute_bind_group,
            render_bind_group_layout,
            render_bind_group,
            atlas_size,
            atlas_texture,
            atlas_view,
            atlas_sampler,
            atlas_allocator: AtlasAllocator::new(atlas_size),
            cull_pipeline,
            finalize_pipeline,
            render_pipeline,
        };

        resources.recreate_bind_groups(device);
        resources
    }

    fn reset_atlas(&mut self, device: &wgpu::Device) {
        self.atlas_texture = create_atlas_texture(device, self.atlas_size);
        self.atlas_view = self
            .atlas_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.atlas_allocator.reset();
        self.recreate_bind_groups(device);
    }

    fn upload_to_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas_entries: &mut HashMap<usize, AtlasEntry>,
        upload: PendingTextureUpload,
    ) {
        if upload.width == 0 || upload.height == 0 {
            atlas_entries.remove(&upload.index);
            return;
        }

        let expected_len = upload
            .width
            .checked_mul(upload.height)
            .and_then(|px| px.checked_mul(4))
            .map(|bytes| bytes as usize)
            .unwrap_or(0);
        if expected_len == 0 || upload.pixels.len() != expected_len {
            atlas_entries.remove(&upload.index);
            return;
        }

        let padded_width = upload.width.saturating_add(ATLAS_PADDING * 2);
        let padded_height = upload.height.saturating_add(ATLAS_PADDING * 2);

        let mut origin = self.atlas_allocator.allocate(padded_width, padded_height);
        if origin.is_none() {
            self.reset_atlas(device);
            atlas_entries.clear();
            origin = self.atlas_allocator.allocate(padded_width, padded_height);
        }

        let Some((slot_x, slot_y)) = origin else {
            atlas_entries.remove(&upload.index);
            return;
        };

        let write_x = slot_x.saturating_add(ATLAS_PADDING);
        let write_y = slot_y.saturating_add(ATLAS_PADDING);

        self.write_rgba_to_atlas(queue, write_x, write_y, upload.width, upload.height, &upload.pixels);

        let inv_atlas = 1.0 / self.atlas_size as f32;
        atlas_entries.insert(
            upload.index,
            AtlasEntry {
                uv_min: [write_x as f32 * inv_atlas, write_y as f32 * inv_atlas],
                uv_max: [
                    (write_x + upload.width) as f32 * inv_atlas,
                    (write_y + upload.height) as f32 * inv_atlas,
                ],
                lod_side: upload.lod_side,
            },
        );
    }

    fn write_rgba_to_atlas(
        &self,
        queue: &wgpu::Queue,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) {
        let bytes_per_pixel = 4usize;
        let row_bytes = width as usize * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let padded_row_bytes = row_bytes.div_ceil(align) * align;

        let (upload_data, bytes_per_row) = if padded_row_bytes == row_bytes {
            (pixels.to_vec(), row_bytes as u32)
        } else {
            let mut padded = vec![0u8; padded_row_bytes * height as usize];
            for row in 0..height as usize {
                let src_start = row * row_bytes;
                let src_end = src_start + row_bytes;
                let dst_start = row * padded_row_bytes;
                let dst_end = dst_start + row_bytes;
                padded[dst_start..dst_end].copy_from_slice(&pixels[src_start..src_end]);
            }
            (padded, padded_row_bytes as u32)
        };

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &upload_data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
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
                    resource: self.indirect_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.meta_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
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
                    resource: self.meta_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.atlas_sampler),
                },
            ],
        });
    }
}

fn lod_rank_for_side(side: u32) -> u8 {
    if side <= 256 {
        0
    } else if side <= 512 {
        1
    } else if side <= 1024 {
        2
    } else {
        3
    }
}

fn create_atlas_texture(device: &wgpu::Device, size: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("masonry_gpu_atlas_texture"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
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
