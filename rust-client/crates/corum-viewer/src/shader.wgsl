struct Camera {
    view_projection: mat4x4<f32>,
    light_direction: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4<f32>(input.position, 1.0);
    output.normal = input.normal;
    output.uv = input.uv;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(input.normal);
    let light = normalize(camera.light_direction.xyz);
    let diffuse = max(dot(normal, light), 0.0);
    let cells = floor(input.uv * 12.0);
    let checker = abs(fract((cells.x + cells.y) * 0.5) * 2.0 - 1.0);
    let color_a = vec3<f32>(0.22, 0.48, 0.78);
    let color_b = vec3<f32>(0.72, 0.86, 0.98);
    let base_color = mix(color_a, color_b, checker);
    return vec4<f32>(base_color * (0.28 + diffuse * 0.72), 1.0);
}
