// UI rendering shader
// Handles glass-effect control buttons with icons

struct ButtonUniform {
    position: vec2<f32>,
    size: vec2<f32>,
    color: vec4<f32>,
    hover: f32,
    button_type: f32, // 0=minimize, 1=maximize, 2=close
    _padding: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> button: ButtonUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    // Transform quad to button position and size
    // Input position is in [-1, 1], need to map to button rect
    let normalized_pos = (in.position + 1.0) * 0.5; // [0, 1]
    let button_pos = button.position + normalized_pos * button.size;
    
    // Convert to clip space.
    // button_pos is normalized [0..1] with origin at top-left.
    let x_clip = button_pos.x * 2.0 - 1.0;
    let y_clip = 1.0 - button_pos.y * 2.0;
    out.clip_position = vec4<f32>(x_clip, y_clip, 0.0, 1.0);
    out.local_pos = in.position;
    
    return out;
}

// Draw rounded rectangle with glass effect
fn rounded_rect(pos: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    let d = abs(pos) - size + radius;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - radius;
}

// Draw minimize icon (horizontal line)
fn draw_minimize_icon(uv: vec2<f32>) -> f32 {
    let icon_size = 0.3;
    let thickness = 0.08;
    
    // Horizontal line
    if abs(uv.x) < icon_size && abs(uv.y) < thickness {
        return 1.0;
    }
    return 0.0;
}

// Draw maximize/restore icon (square outline)
fn draw_maximize_icon(uv: vec2<f32>) -> f32 {
    let icon_size = 0.25;
    let thickness = 0.06;
    
    // Square outline
    let outer = max(abs(uv.x), abs(uv.y)) < icon_size;
    let inner = max(abs(uv.x), abs(uv.y)) < icon_size - thickness;
    
    if outer && !inner {
        return 1.0;
    }
    return 0.0;
}

// Draw close icon (X)
fn draw_close_icon(uv: vec2<f32>) -> f32 {
    let icon_size = 0.28;
    let thickness = 0.08;
    
    // Two diagonal lines forming X
    let d1 = abs(uv.x - uv.y);
    let d2 = abs(uv.x + uv.y);
    
    if (d1 < thickness || d2 < thickness) && max(abs(uv.x), abs(uv.y)) < icon_size {
        return 1.0;
    }
    return 0.0;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.local_pos;
    
    // Rounded rectangle background
    let rect_dist = rounded_rect(uv, vec2<f32>(0.9, 0.9), 0.2);
    
    if rect_dist > 0.0 {
        discard;
    }
    
    // Glass effect - semi-transparent with blur simulation
    var base_color = button.color;
    
    // Add subtle gradient for depth
    let gradient = 1.0 - (uv.y + 1.0) * 0.1;
    base_color = vec4<f32>(base_color.rgb * gradient, base_color.a);
    
    // Hover effect - brighten
    if button.hover > 0.5 {
        base_color = vec4<f32>(base_color.rgb * 1.3, base_color.a);
    }
    
    // Add frosted glass edge highlight
    let edge_dist = abs(rect_dist);
    if edge_dist < 0.1 {
        let edge_factor = 1.0 - edge_dist / 0.1;
        base_color = vec4<f32>(
            base_color.rgb + vec3<f32>(0.3) * edge_factor * 0.5,
            base_color.a
        );
    }
    
    // Draw icon based on button type
    var icon_color = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    var icon_alpha = 0.0;
    
    if button.button_type < 0.5 {
        // Minimize
        icon_alpha = draw_minimize_icon(uv);
    } else if button.button_type < 1.5 {
        // Maximize
        icon_alpha = draw_maximize_icon(uv);
    } else {
        // Close
        icon_alpha = draw_close_icon(uv);
    }
    
    // Blend icon with background
    if icon_alpha > 0.5 {
        return icon_color;
    }
    
    return base_color;
}
