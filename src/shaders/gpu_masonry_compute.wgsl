struct Item {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
    target_texture_side: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

struct CullMeta {
    item_count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    viewport_min: vec2<f32>,
    viewport_max: vec2<f32>,
    screen_size: vec2<f32>,
    _pad3: vec2<f32>,
};

struct Counter {
    visible_count: atomic<u32>,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0)
var<storage, read> items: array<Item>;

@group(0) @binding(1)
var<storage, read_write> visible_indices: array<u32>;

@group(0) @binding(2)
var<storage, read_write> visible_lod: array<u32>;

@group(0) @binding(3)
var<storage, read_write> indirect_args: array<u32>;

@group(0) @binding(4)
var<uniform> meta: CullMeta;

@group(0) @binding(5)
var<storage, read_write> counter: Counter;

@compute @workgroup_size(64)
fn cull_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x;
    if (index >= meta.item_count) {
        return;
    }

    let item = items[index];
    let intersects = item.max_x > meta.viewport_min.x && item.min_x < meta.viewport_max.x &&
        item.max_y > meta.viewport_min.y && item.min_y < meta.viewport_max.y;

    if (intersects) {
        let out_index = atomicAdd(&counter.visible_count, 1u);
        if (out_index < meta.item_count) {
            visible_indices[out_index] = index;
            visible_lod[out_index] = item.target_texture_side;
        }
    }
}

@compute @workgroup_size(1)
fn finalize_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x != 0u) {
        return;
    }

    let visible_count = atomicLoad(&counter.visible_count);

    indirect_args[0] = 6u;
    indirect_args[1] = visible_count;
    indirect_args[2] = 0u;
    indirect_args[3] = 0u;
}