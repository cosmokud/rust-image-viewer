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

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) lod_hint: f32,
};

@group(0) @binding(0)
var<storage, read> items: array<Item>;

@group(0) @binding(1)
var<storage, read> visible_indices: array<u32>;

@group(0) @binding(2)
var<storage, read> visible_lod: array<u32>;

@group(0) @binding(3)
var<uniform> meta: CullMeta;

var<private> unit_quad: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 1.0),
);

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VsOut {
    var out: VsOut;

    let item_index = visible_indices[instance_index];
    let item = items[item_index];

    let item_min = vec2<f32>(item.min_x, item.min_y);
    let item_size = vec2<f32>(
        max(item.max_x - item.min_x, 0.0),
        max(item.max_y - item.min_y, 0.0),
    );

    let pixel_pos = item_min + item_size * unit_quad[vertex_index];
    let safe_screen = max(meta.screen_size, vec2<f32>(1.0, 1.0));
    let ndc = vec2<f32>(
        (pixel_pos.x / safe_screen.x) * 2.0 - 1.0,
        1.0 - (pixel_pos.y / safe_screen.y) * 2.0,
    );

    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.lod_hint = f32(visible_lod[instance_index]);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let _lod = in.lod_hint;
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}