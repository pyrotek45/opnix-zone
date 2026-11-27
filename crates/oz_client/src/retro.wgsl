// Ordered-dither colour quantisation, applied after tonemapping.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Retro {
    enabled: f32,
    levels: f32,
    strength: f32,
    aberration: f32,
    edge_r: f32,
    edge_g: f32,
    edge_b: f32,
    edge_a: f32,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> retro: Retro;

fn bayer(p: vec2<u32>) -> f32 {
    var pattern = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return (pattern[(p.y % 4u) * 4u + p.x % 4u] + 0.5) / 16.0 - 0.5;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let centered = in.uv * 2.0 - 1.0;
    let reach = dot(centered, centered);
    var rgb = textureSample(scene, scene_sampler, in.uv).rgb;
    // Impact fringing: red and blue pulled apart toward the edges, nothing at the centre.
    if retro.aberration > 0.001 {
        let shift = centered * retro.aberration * 0.012 * reach;
        rgb.r = textureSample(scene, scene_sampler, in.uv + shift).r;
        rgb.b = textureSample(scene, scene_sampler, in.uv - shift).b;
    }
    // An edge glow (kill confirms, big hits) and a darker vignette while it pulses.
    let edge = retro.edge_a * smoothstep(0.45, 1.9, reach);
    rgb = mix(rgb, vec3<f32>(retro.edge_r, retro.edge_g, retro.edge_b), clamp(edge, 0.0, 0.85));
    let vignette = 1.0 - (0.14 + 0.25 * retro.aberration) * smoothstep(0.35, 1.75, reach);
    if retro.enabled < 0.5 {
        return vec4<f32>(rgb * vignette, 1.0);
    }
    // Quantise in perceptual rather than linear light.
    let srgb = pow(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    let n = max(2.0, retro.levels);
    let cell = vec2<u32>(in.position.xy);
    let q = clamp(floor(srgb * n + 0.5 + bayer(cell) * retro.strength) / n, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(pow(q, vec3<f32>(2.2)) * vignette, 1.0);
}
