const MAX_POINT_LIGHTS: u32 = 256u;

// Blend classes stored per vertex (see `BLEND_*` in the sandbox).
const CLASS_OPAQUE: f32 = 0.0;
const CLASS_ALPHA: f32 = 1.0;
const CLASS_ADDITIVE: f32 = 2.0;

struct Camera {
    view_projection: mat4x4<f32>,
    // xyz = directional light, w = gain applied to baked lighting (VCL and lightmaps)
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
@group(1) @binding(2)
var lightmaps: texture_2d_array<f32>;
@group(1) @binding(3)
var lightmap_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) layer: f32,
    @location(5) baked: f32,
    @location(6) lightmap_uv: vec2<f32>,
    @location(7) lightmap_layer: f32,
    @location(8) blend: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) layer: f32,
    @location(4) world_position: vec3<f32>,
    @location(5) @interpolate(flat) baked: f32,
    @location(6) lightmap_uv: vec2<f32>,
    @location(7) @interpolate(flat) lightmap_layer: f32,
    @location(8) @interpolate(flat) blend: f32,
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
    output.lightmap_uv = input.lightmap_uv;
    output.lightmap_layer = input.lightmap_layer;
    output.blend = input.blend;
    return output;
}

// Sum of the map's coloured point lights, with a smooth falloff to zero at each light's radius.
// The scenery already has these lights baked in (VCL and lightmaps), so this only lights actors.
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

// Colour of the fragment for the classes that are lit (opaque and alpha-blended).
fn shade(input: VertexOutput, texel: vec4<f32>, light: vec4<f32>, textured: bool) -> vec3<f32> {
    let gain = camera.light_direction.w;

    // Baked vertex colours (VCL) already contain the lighting: texture x VCL, nothing added.
    if input.baked > 0.5 {
        return texel.rgb * input.color * gain;
    }
    // Baked lightmap: texture x lightmap.
    if input.lightmap_layer >= 0.0 {
        return texel.rgb * light.rgb * gain;
    }

    let normal = normalize(input.normal);
    let diffuse = max(dot(normal, normalize(camera.light_direction.xyz)), 0.0);
    var albedo = input.color;
    var lighting = vec3<f32>(0.30 + diffuse * 0.70);
    if textured {
        albedo = texel.rgb * input.color;
        lighting = vec3<f32>(0.62 + diffuse * 0.38);
    }
    lighting = lighting + point_light_contribution(input.world_position, normal) * 1.5;
    // Flat colours and the lighting model above were tuned as linear values; encode them for
    // the gamma-space surface. Textures already are gamma-space, so only the light is encoded.
    if textured {
        return albedo * pow(lighting, vec3<f32>(1.0 / 2.2));
    }
    return pow(albedo * lighting, vec3<f32>(1.0 / 2.2));
}

// Each pipeline draws the same buffers and keeps only the vertices of its own class.

@fragment
fn fragment_opaque(input: VertexOutput) -> @location(0) vec4<f32> {
    // Sample unconditionally so derivatives stay in uniform control flow.
    let texel = textureSample(map_textures, map_sampler, input.uv, i32(max(input.layer, 0.0) + 0.5));
    let light = textureSample(
        lightmaps,
        lightmap_sampler,
        input.lightmap_uv,
        i32(max(input.lightmap_layer, 0.0) + 0.5),
    );
    let textured = input.layer >= 0.0;
    if input.blend != CLASS_OPAQUE {
        discard;
    }
    // Hard cut-outs (leaves, grass, fences).
    if textured && texel.a < 0.5 {
        discard;
    }
    return vec4<f32>(shade(input, texel, light, textured), 1.0);
}

@fragment
fn fragment_alpha(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(map_textures, map_sampler, input.uv, i32(max(input.layer, 0.0) + 0.5));
    let light = textureSample(
        lightmaps,
        lightmap_sampler,
        input.lightmap_uv,
        i32(max(input.lightmap_layer, 0.0) + 0.5),
    );
    if input.blend != CLASS_ALPHA || texel.a < 0.01 {
        discard;
    }
    // Translucent surfaces (water, glass): lit like the rest, mixed with what is behind.
    return vec4<f32>(shade(input, texel, light, input.layer >= 0.0), texel.a);
}

@fragment
fn fragment_additive(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(map_textures, map_sampler, input.uv, i32(max(input.layer, 0.0) + 0.5));
    if input.blend != CLASS_ADDITIVE {
        discard;
    }
    // Light-emitting effects (fire, glows, waterfalls): never shaded, added to the frame.
    return vec4<f32>(texel.rgb * input.color, texel.a);
}
