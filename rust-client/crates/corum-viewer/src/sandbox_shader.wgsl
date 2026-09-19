const MAX_POINT_LIGHTS: u32 = 32u;

struct Camera {
    view_projection: mat4x4<f32>,
    light_direction: vec4<f32>,
};

struct PointLight {
    // xyz = position, w = radius
    position_radius: vec4<f32>,
    color: vec4<f32>,
};

struct Lights {
    count: vec4<u32>,
    items: array<PointLight, MAX_POINT_LIGHTS>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;
@group(0) @binding(1)
var<uniform> lights: Lights;

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
    @location(5) baked: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) layer: f32,
    @location(4) world_position: vec3<f32>,
    @location(5) @interpolate(flat) baked: f32,
};

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4<f32>(input.position, 1.0);
    output.normal = input.normal;
    output.color = input.color;
    output.uv = input.uv;
    output.layer = input.layer;
    output.world_position = input.position;
    output.baked = input.baked;
    return output;
}

// Sum of the map's coloured point lights, with a smooth falloff to zero at each light's radius.
fn point_light_contribution(position: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    var total = vec3<f32>(0.0);
    for (var index = 0u; index < lights.count.x; index = index + 1u) {
        let light = lights.items[index];
        let to_light = light.position_radius.xyz - position;
        let distance = length(to_light);
        let falloff = clamp(1.0 - distance / max(light.position_radius.w, 0.001), 0.0, 1.0);
        let facing = 0.35 + 0.65 * max(dot(normal, to_light / max(distance, 0.001)), 0.0);
        total = total + light.color.rgb * (falloff * falloff * facing);
    }
    return total;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Sample unconditionally so derivatives stay in uniform control flow.
    let texel = textureSample(map_textures, map_sampler, input.uv, i32(max(input.layer, 0.0) + 0.5));
    let textured = input.layer >= 0.0;
    if textured && texel.a < 0.5 {
        discard;
    }
    let normal = normalize(input.normal);
    let diffuse = max(dot(normal, normalize(camera.light_direction.xyz)), 0.0);

    // Baked vertex colours already contain the lighting: texture x VCL, nothing added.
    if input.baked > 0.5 {
        return vec4<f32>(texel.rgb * input.color, 1.0);
    }

    var albedo = input.color;
    var lighting = vec3<f32>(0.30 + diffuse * 0.70);
    if textured {
        albedo = texel.rgb * input.color;
        lighting = vec3<f32>(0.62 + diffuse * 0.38);
    }
    lighting = lighting + point_light_contribution(input.world_position, normal) * 1.5;
    return vec4<f32>(albedo * lighting, 1.0);
}
