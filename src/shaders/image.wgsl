// Image rendering shader
// Handles image display with zoom, pan, rotation, and opacity

struct Uniforms {
    scale: vec2<f32>,
    translate: vec2<f32>,
    rotation: f32,
    opacity: f32,
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

    // Rotate first, then apply the (non-uniform) fit/zoom scale in screen axes.
    // This avoids stretching when rotation is 90°/270°.
    let c = cos(uniforms.rotation);
    let s = sin(uniforms.rotation);
    let rotated = vec2<f32>(in.position.x * c - in.position.y * s, in.position.x * s + in.position.y * c);
    let scaled = rotated * uniforms.scale;
    let final_pos = scaled + uniforms.translate;
    out.clip_position = vec4<f32>(final_pos, 0.0, 1.0);
    out.tex_coords = in.tex_coords;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    
    // Apply opacity for fade animations
    return vec4<f32>(color.rgb, color.a * uniforms.opacity);
}
