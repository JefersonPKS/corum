struct Camera {
    view_projection: mat4x4<f32>,
    light_direction: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(1) @binding(0)
var map_textures: texture_2d_array<f32>;
@group(1) @binding(1)
var map_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) layer: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) layer: f32,
};

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4<f32>(input.position, 1.0);
    output.normal = input.normal;
    output.color = input.color;
    output.uv = input.uv;
    output.layer = input.layer;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Sample unconditionally so derivatives stay in uniform control flow.
    let texel = textureSample(map_textures, map_sampler, input.uv, i32(max(input.layer, 0.0) + 0.5));
    let textured = input.layer >= 0.0;
    if textured && texel.a < 0.5 {
        discard;
    }
    let diffuse = max(dot(normalize(input.normal), normalize(camera.light_direction.xyz)), 0.0);
    var albedo = input.color;
    var lighting = 0.30 + diffuse * 0.70;
    if textured {
        albedo = texel.rgb * input.color;
        lighting = 0.62 + diffuse * 0.38;
    }
    return vec4<f32>(albedo * lighting, 1.0);
}
