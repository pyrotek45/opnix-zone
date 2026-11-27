//! Full-screen ordered colour quantisation, after tonemapping and before UI.
//!
//! The world camera renders to a low-resolution target (see `view`), so this
//! pass runs on very few pixels and the chunky look is nearly free.

#![allow(dead_code)] // The `ShaderType` derive emits helper functions that are never called.

use bevy::{
    asset::{load_internal_asset, weak_handle},
    core_pipeline::{
        core_3d::graph::{Core3d, Node3d},
        fullscreen_vertex_shader::fullscreen_shader_vertex_state,
    },
    ecs::query::QueryItem,
    prelude::*,
    render::{
        RenderApp,
        extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin},
        render_graph::{NodeRunError, RenderGraphApp, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner},
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice},
        view::ViewTarget,
    },
};

const RETRO_SHADER: Handle<Shader> = weak_handle!("5c3b3c5e-8f2a-4f4e-9a57-0d1f6f1e7a21");

/// Add to a 3D camera to enable the retro pass.
#[derive(Component, Clone, Copy, ExtractComponent, ShaderType)]
pub struct Retro {
    pub enabled: f32,
    pub levels: f32,
    pub strength: f32,
    /// Colour fringing at the screen edges, 0..1 (a big hit or a blast nearby; see `punch`).
    pub aberration: f32,
    /// A coloured glow washing in from the screen edges (`edge_a` = strength).
    pub edge_r: f32,
    pub edge_g: f32,
    pub edge_b: f32,
    pub edge_a: f32,
}

impl Default for Retro {
    fn default() -> Self {
        Self { enabled: 1.0, levels: 8.0, strength: 0.85, aberration: 0.0, edge_r: 0.0, edge_g: 0.0, edge_b: 0.0, edge_a: 0.0 }
    }
}

pub struct RetroPlugin;

impl Plugin for RetroPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, RETRO_SHADER, "retro.wgsl", Shader::from_wgsl);
        app.add_plugins((ExtractComponentPlugin::<Retro>::default(), UniformComponentPlugin::<Retro>::default()));
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .add_render_graph_node::<ViewNodeRunner<RetroNode>>(Core3d, RetroLabel)
                .add_render_graph_edges(Core3d, (Node3d::Tonemapping, RetroLabel, Node3d::EndMainPassPostProcessing));
        }
    }

    fn finish(&self, app: &mut App) {
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.init_resource::<RetroPipeline>();
        }
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct RetroLabel;

#[derive(Default)]
struct RetroNode;

#[derive(Resource)]
struct RetroPipeline {
    layout: BindGroupLayout,
    sampler: Sampler,
    id: CachedRenderPipelineId,
}

impl FromWorld for RetroPipeline {
    fn from_world(world: &mut World) -> Self {
        let device = world.resource::<RenderDevice>();
        let layout = device.create_bind_group_layout(
            "retro-layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (texture_2d(TextureSampleType::Float { filterable: true }), sampler(SamplerBindingType::Filtering), uniform_buffer::<Retro>(true)),
            ),
        );
        let sampler = device.create_sampler(&SamplerDescriptor::default());
        let id = world.resource_mut::<PipelineCache>().queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("retro".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen_shader_vertex_state(),
            fragment: Some(FragmentState {
                shader: RETRO_SHADER,
                shader_defs: vec![],
                entry_point: "fragment".into(),
                targets: vec![Some(ColorTargetState { format: TextureFormat::bevy_default(), blend: None, write_mask: ColorWrites::ALL })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            push_constant_ranges: vec![],
            zero_initialize_workgroup_memory: false,
        });
        Self { layout, sampler, id }
    }
}

impl ViewNode for RetroNode {
    type ViewQuery = (&'static ViewTarget, &'static DynamicUniformIndex<Retro>);

    fn run(&self, _: &mut RenderGraphContext, ctx: &mut RenderContext, (target, index): QueryItem<Self::ViewQuery>, world: &World) -> Result<(), NodeRunError> {
        let data = world.resource::<RetroPipeline>();
        let Some(pipeline) = world.resource::<PipelineCache>().get_render_pipeline(data.id) else { return Ok(()) };
        let Some(uniform) = world.resource::<ComponentUniforms<Retro>>().uniforms().binding() else { return Ok(()) };
        let buffers = target.post_process_write();
        let bindings = ctx.render_device().create_bind_group("retro-bindings", &data.layout, &BindGroupEntries::sequential((buffers.source, &data.sampler, uniform.clone())));
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("retro-pass"),
            color_attachments: &[Some(RenderPassColorAttachment { view: buffers.destination, resolve_target: None, ops: Operations::default() })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_render_pipeline(pipeline);
        pass.set_bind_group(0, &bindings, &[index.index()]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}
