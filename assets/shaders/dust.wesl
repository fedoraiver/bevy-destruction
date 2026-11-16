import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
    view_transformations::depth_ndc_to_view_z,
};
@if(DEPTH_PREPASS)
import bevy_pbr::prepass::utils::prepass_depth;

struct DustSettings {
    color: vec4<f32>,
    time: vec4<f32>,
    shape: vec4<f32>,
    light: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> dust: DustSettings;
@group(constants::MATERIAL_BIND_GROUP) @binding(1)
var atlas: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2)
var atlas_sampler: sampler;

fn frame_sample(uv: vec2<f32>, frame: f32, variant: f32) -> vec4<f32> {
    let inset_uv = mix(vec2<f32>(0.5 / 64.0), vec2<f32>(1.0 - 0.5 / 64.0), uv);
    let atlas_uv = (inset_uv + vec2<f32>(frame, variant)) / vec2<f32>(8.0, 4.0);
    return textureSample(atlas, atlas_sampler, atlas_uv);
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    let progress = clamp(dust.time.x / max(dust.time.y, 0.001), 0.0, 1.0);
    let sine = sin(dust.shape.x);
    let cosine = cos(dust.shape.x);
    let centred = in.uv - vec2<f32>(0.5);
    let uv = vec2<f32>(cosine * centred.x - sine * centred.y, sine * centred.x + cosine * centred.y) + vec2<f32>(0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) { discard; }
    let frame = progress * 7.0;
    let first = floor(frame);
    let sample = mix(frame_sample(uv, first, dust.time.z), frame_sample(uv, min(first + 1.0, 7.0), dust.time.z), fract(frame));
    let particle_distance = -depth_ndc_to_view_z(in.position.z);
    var depth_fade = 1.0;
    @if(DEPTH_PREPASS) {
        let scene_depth = prepass_depth(in.position, 0u);
        if scene_depth > 0.0 {
            let scene_distance = -depth_ndc_to_view_z(scene_depth);
            depth_fade = clamp((scene_distance - particle_distance) / max(dust.shape.y, 0.001), 0.0, 1.0);
        }
    }
    let near_fade = smoothstep(dust.shape.z, dust.shape.w, particle_distance);
    let envelope = smoothstep(0.0, 0.06, progress) * pow(1.0 - progress, 0.9);
    let alpha = sample.a * dust.time.w * envelope * depth_fade * near_fade;
    if alpha < 0.002 { discard; }

    let normal_xy = sample.rg * 2.0 - vec2<f32>(1.0);
    let unrolled = vec2<f32>(cosine * normal_xy.x + sine * normal_xy.y, -sine * normal_xy.x + cosine * normal_xy.y);
    let normal_z = sqrt(max(1.0 - dot(unrolled, unrolled), 0.02));
    let normal_world = normalize(view.world_from_view[0].xyz * unrolled.x - view.world_from_view[1].xyz * unrolled.y + view.world_from_view[2].xyz * normal_z);
    let diffuse = max(dot(normal_world, normalize(dust.light.xyz)), 0.0);
    let light = dust.light.w + diffuse * 0.65;
    let density_shade = mix(1.08, 0.74, sample.b);
    var out: FragmentOutput;
    out.color = vec4<f32>(dust.color.rgb * light * density_shade, alpha);
    return out;
}
