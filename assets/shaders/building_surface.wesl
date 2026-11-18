import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    mesh_functions::get_tag,
};

@if(PREPASS_PIPELINE)
import bevy_pbr::{
    prepass::io::{VertexOutput, FragmentOutput},
    deferred::functions::deferred_output,
};
@else
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};

struct SurfaceSettings {
    grid: vec4<f32>,
    fracture: vec4<f32>,
    fracture_color: vec4<f32>,
    fracture_surface: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> surface: SurfaceSettings;

@group(constants::MATERIAL_BIND_GROUP) @binding(101)
var<storage, read> contact_mask: array<vec4<f32>>;

fn inside_contact(position: vec3<f32>, encoded_header: f32) -> bool {
    if encoded_header < 0.5 {
        return false;
    }
    let header = u32(round(encoded_header));
    let count = u32(contact_mask[header].x);
    for (var i = 0u; i < count; i += 1u) {
        let polygon = u32(contact_mask[header + 1u + i].x);
        let info = contact_mask[polygon];
        let relative = position - info.xyz;
        var inside = true;
        for (var edge = 0u; edge < u32(info.w); edge += 1u) {
            let halfspace = contact_mask[polygon + 1u + edge];
            if dot(halfspace.xyz, relative) + halfspace.w < -0.0000005 {
                inside = false;
                break;
            }
        }
        if inside {
            return true;
        }
    }
    return false;
}

fn hash3(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.11369, 0.13787));
    let r = q + vec3<f32>(dot(q, q.yzx + vec3<f32>(19.19)));
    return fract((r.x + r.y) * r.z);
}

fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (vec3<f32>(3.0) - 2.0 * f);
    let a = mix(hash3(i), hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

fn height(p: vec3<f32>) -> f32 {
    let aggregate = noise3(p * surface.fracture.z);
    let pores = noise3(p * surface.fracture.w + vec3<f32>(7.3, 1.7, 9.1));
    return surface.fracture.y * (aggregate + 0.16 * pores);
}

fn grid_coverage(position: vec2<f32>, spacing: f32, width: f32) -> f32 {
    let cell = position / spacing;
    let footprint = max(fwidth(cell), vec2<f32>(0.00001));
    let distance_pixels = abs(fract(cell + vec2<f32>(0.5)) - vec2<f32>(0.5)) / footprint;
    let distance_line = min(distance_pixels.x, distance_pixels.y);
    let line = 1.0 - smoothstep(max(width * 0.5 - 0.5, 0.0), width * 0.5 + 0.5, distance_line);
    let pixels_per_cell = 1.0 / max(footprint.x, footprint.y);
    return line * smoothstep(1.5, 3.5, pixels_per_cell);
}

fn unit(v: vec3<f32>) -> vec3<f32> {
    return v / max(length(v), 0.000000000001);
}

fn heat_color(color: vec3<f32>, tag: u32) -> vec3<f32> {
    var tint: vec3<f32>;
    if tag == u32(constants::HEAT_AMBER_TAG) {
        tint = vec3<f32>(1.0, 0.36, 0.015);
    } else if tag == u32(constants::HEAT_RED_TAG) {
        tint = vec3<f32>(1.0, 0.025, 0.015);
    } else {
        return color;
    }
    let detail = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(color, tint * detail * 1.5, 0.85);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    let position = vec3<f32>(in.uv, in.uv_b.x);
    let rest_dx = dpdx(position);
    let rest_dy = dpdy(position);
    let world_dx = dpdx(in.world_position.xyz);
    let world_dy = dpdy(in.world_position.xyz);
    let normal = unit(cross(rest_dx, rest_dy));

    // Derivatives must precede the per-fragment contact mask branch.
    let horizontal = cross(vec3<f32>(0.0, 1.0, 0.0), normal);
    let wall_tangent = horizontal / max(length(horizontal), 0.00001);
    let tangent = select(vec3<f32>(1.0, 0.0, 0.0), wall_tangent, abs(normal.y) < 0.999);
    let bitangent = select(vec3<f32>(0.0, 0.0, 1.0), normalize(cross(normal, tangent)), abs(normal.y) < 0.999);
    let plane_position = vec2<f32>(dot(position, tangent), dot(position, bitangent));
    let minor = grid_coverage(plane_position, surface.grid.x, surface.grid.z);
    let major = grid_coverage(plane_position, surface.grid.y, surface.grid.w);

    if surface.fracture.x < 0.5 && !inside_contact(position, in.uv_b.y) {
        let with_minor = mix(pbr.material.base_color.rgb, vec3<f32>(0.34, 0.39, 0.41), minor * (1.0 - major) * 0.62);
        pbr.material.base_color = vec4<f32>(mix(with_minor, vec3<f32>(0.065, 0.085, 0.095), major * 0.88), 1.0);
    } else {
        pbr.material.perceptual_roughness = surface.fracture_surface.x;
        pbr.material.metallic = surface.fracture_surface.y;
        let aggregate = noise3(position * surface.fracture.z);
        let pores = noise3(position * surface.fracture.w + vec3<f32>(7.3, 1.7, 9.1));
        let variation = 0.72 + 0.40 * aggregate - 0.13 * smoothstep(0.67, 0.91, pores);
        var fracture_color = surface.fracture_color.rgb * variation;
        @if(VERTEX_COLORS) {
            if surface.fracture.x > 1.5 {
                let depth = clamp(in.color.r, 0.0, 1.0);
                let fresh = mix(1.22, 0.66, smoothstep(0.1, 1.0, depth));
                let aggregate_tint = mix(0.9, 1.12, in.color.g);
                fracture_color = fracture_color * fresh * aggregate_tint;
                pbr.material.perceptual_roughness = max(pbr.material.perceptual_roughness, 0.94);
            }
        }
        pbr.material.base_color = vec4<f32>(fracture_color, 1.0);
        let e = 0.001;
        let gradient = vec3<f32>(
            height(position + vec3<f32>(e, 0.0, 0.0)) - height(position - vec3<f32>(e, 0.0, 0.0)),
            height(position + vec3<f32>(0.0, e, 0.0)) - height(position - vec3<f32>(0.0, e, 0.0)),
            height(position + vec3<f32>(0.0, 0.0, e)) - height(position - vec3<f32>(0.0, 0.0, e))
        ) / (2.0 * e);
        let rest_tangent = unit(rest_dx);
        let rest_bitangent = cross(normal, rest_tangent);
        let world_tangent = unit(world_dx);
        let world_bitangent = cross(unit(cross(world_dx, world_dy)), world_tangent);
        let world_gradient = world_tangent * dot(gradient, rest_tangent)
            + world_bitangent * dot(gradient, rest_bitangent);
        pbr.N = normalize(pbr.N - world_gradient);
    }

    pbr.material.base_color = vec4<f32>(
        heat_color(pbr.material.base_color.rgb, get_tag(in.instance_index)),
        pbr.material.base_color.a,
    );
    pbr.material.base_color = alpha_discard(pbr.material.flags, pbr.material.alpha_cutoff, pbr.material.base_color);
    var out: FragmentOutput;
    @if(PREPASS_PIPELINE) {
        out = deferred_output(in, pbr);
    } @else {
        out.color = apply_pbr_lighting(pbr);
        out.color = main_pass_post_lighting_processing(pbr, out.color);
    }
    return out;
}
