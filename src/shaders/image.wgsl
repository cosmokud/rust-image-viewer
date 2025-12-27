// Image rendering shader
// Handles image display with zoom, pan, rotation, and opacity

struct Uniforms {
    transform: mat4x4<f32>,
    opacity: f32,
    rotation: f32,
    _padding: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(1) @binding(1)
var s_diffuse: sampler;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    // Apply transform
    let pos = uniforms.transform * vec4<f32>(in.position, 0.0, 1.0);
    out.clip_position = pos;
    out.tex_coords = in.tex_coords;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    
    // Apply opacity for fade animations
    return vec4<f32>(color.rgb, color.a * uniforms.opacity);
}
