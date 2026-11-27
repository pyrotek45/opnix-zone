//! The static world: terrain, sea, props, sky, and the day/night lighting.
//!
//! Everything here is built once per match from the map recipe. Terrain is
//! split into chunks and props are merged into one mesh per 64 m region, so
//! the whole island costs a few dozen draw calls and culls well.

use crate::{
    ClientSet,
    art::{Art, MeshBuilder},
    settings::Settings,
    view::{Flashlight, WorldCamera, spawn_camera},
};
use bevy::{
    asset::RenderAssetUsages,
    core_pipeline::Skybox,
    pbr::{CascadeShadowConfigBuilder, NotShadowCaster},
    platform::collections::HashMap,
    prelude::*,
    render::{
        mesh::PrimitiveTopology,
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension},
    },
};
use oz_core::{
    model::Part,
    worldgen::{Prop, PropKind, sky_cubemap},
};
use oz_sim::prelude::*;
use std::f32::consts::TAU;

const CHUNK_CELLS: usize = 32;
const PROP_REGION: f32 = 64.0;

#[derive(Resource)]
pub struct Sky {
    pub day: Handle<Image>,
    pub night: Handle<Image>,
    fog: Vec3,
    water: Vec3,
    water_material: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Sun;

pub struct ScenePlugin;

/// Label for building the static world at match start.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct BuildScene;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(MatchPhase::Active), (build_scene, spawn_camera).chain().in_set(BuildScene))
            .add_systems(Update, day_night.in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

fn cubemap(pixels: Vec<u8>, size: u32) -> Image {
    let mut image = Image::new(Extent3d { width: size, height: size, depth_or_array_layers: 6 }, TextureDimension::D2, pixels, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    image
}

/// Block model for one prop, in prop-local space.
fn prop_model(prop: &Prop) -> BlockModel {
    let s = prop.scale;
    match prop.kind {
        PropKind::Palm => {
            let mut model = BlockModel::new().cylinder(Paint::Wood, [0.0, s * 0.5, 0.0], [0.32, s, 0.32]);
            for n in 0..6 {
                let angle = n as f32 * TAU / 6.0;
                let dir = Vec3::new(angle.cos(), 0.1, angle.sin()).normalize();
                model.parts.push(Part {
                    shape: Shape::Cone,
                    paint: if n % 2 == 0 { Paint::Leaf } else { Paint::LeafLight },
                    pos: Vec3::Y * s + dir * 0.8,
                    size: Vec3::new(1.0, 2.5, 0.5),
                    rot: Quat::from_rotation_arc(Vec3::Y, dir),
                    anim: Anim::None,
                });
            }
            model
        }
        PropKind::Rock => BlockModel::new().shard(Paint::Rock, [0.0, s * 0.24, 0.0], [s, s * 0.65, s * 0.8]),
        PropKind::Ruin => BlockModel::new().cube(Paint::Concrete, [0.0, 0.9 * s, 0.0], [3.2 * s, 1.8 * s, 0.7 * s]).cube(Paint::Rust, [0.0, 1.84 * s, 0.0], [3.3 * s, 0.12, 0.8 * s]),
        PropKind::Pylon => BlockModel::new().cube(Paint::Concrete, [0.0, 1.9 * s, 0.0], [1.4, 3.8 * s, 1.4]).cube(Paint::Teal, [0.0, 3.8 * s + 0.08, 0.0], [1.5, 0.16, 1.5]).cube(
            Paint::Glow,
            [0.0, 2.6 * s, -0.71],
            [0.5, 0.12, 0.03],
        ),
        PropKind::Crate => BlockModel::new().cube(Paint::Teal, [0.0, 0.55, 0.0], [1.25, 1.1, 1.25]).cube(Paint::Gold, [0.0, 0.56, 0.636], [0.84, 0.16, 0.035]),
        PropKind::Tree => {
            // A trunk with a branch, and a crown of chunky leaf clusters.
            let mut model = BlockModel::new().cylinder(Paint::Wood, [0.0, s * 0.35, 0.0], [0.34, s * 0.7, 0.34]).segment(Paint::Wood, [0.0, s * 0.5, 0.0], [0.6, s * 0.75, 0.2], 0.14);
            for n in 0..7 {
                let angle = n as f32 * 2.39996 + prop.yaw;
                let r = if n == 0 { 0.0 } else { s * 0.28 };
                let at = [angle.cos() * r, s * (0.82 + 0.1 * (n % 3) as f32), angle.sin() * r];
                let size = s * (0.42 - 0.03 * (n % 3) as f32);
                model = model.shard(if n % 3 == 0 { Paint::LeafLight } else { Paint::Leaf }, at, [size, size * 0.8, size]);
            }
            model
        }
        PropKind::Bush => BlockModel::new().shard(Paint::Leaf, [0.0, 0.45 * s, 0.0], [1.0 * s, 0.7 * s, 0.9 * s]).shard(Paint::LeafLight, [0.35 * s, 0.6 * s, 0.2], [0.6 * s, 0.5 * s, 0.6 * s]),
        PropKind::Lamp => {
            BlockModel::new().cube(Paint::Dark, [0.0, 1.9, 0.0], [0.12, 3.8, 0.12]).cube(Paint::Dark, [0.25, 3.75, 0.0], [0.6, 0.08, 0.1]).cube(Paint::Glow, [0.45, 3.65, 0.0], [0.28, 0.1, 0.18])
        }
    }
}

fn build_scene(
    mut commands: Commands,
    map: Res<Map>,
    rules: Res<Rules>,
    content: Res<Content>,
    art: Res<Art>,
    settings: Res<Settings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let look = rules.recipe.biome.look();

    // Terrain chunks.
    let chunks = map.chunks_per_side(CHUNK_CELLS);
    for cz in 0..chunks {
        for cx in 0..chunks {
            let chunk = map.chunk(cx, cz, CHUNK_CELLS);
            if chunk.positions.is_empty() {
                continue;
            }
            let mesh = Mesh::new(PrimitiveTopology::TriangleList, crate::art::KEEP_FOR_BOUNDS)
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, chunk.positions)
                .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, chunk.normals)
                .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, chunk.colors);
            commands.spawn((MatchEntity, Mesh3d(meshes.add(mesh)), MeshMaterial3d(art.lit.clone()), Transform::IDENTITY));
        }
    }

    // The sea: one big unlit slab whose colour follows the time of day.
    let water_material = materials.add(StandardMaterial { base_color: Color::srgb_from_array(look.water), unlit: true, ..default() });
    let extent = rules.recipe.half_extent * 2.0 + 900.0;
    commands.spawn((
        MatchEntity,
        Mesh3d(art.primitive(Shape::Cube)),
        MeshMaterial3d(water_material.clone()),
        Transform::from_xyz(0.0, map.water - 0.1, 0.0).with_scale(Vec3::new(extent, 0.2, extent)),
        NotShadowCaster,
    ));

    // Props, merged per region; glowing bits go to the unlit mesh.
    let mut regions: HashMap<(i32, i32), (MeshBuilder, MeshBuilder)> = HashMap::default();
    for prop in &map.props {
        // Bushes stand on their own so they can shake when someone pushes through (see `bushes`).
        if matches!(prop.kind, PropKind::Bush) {
            let baked = art.bake(&prop_model(prop), &mut meshes);
            let entity = commands
                .spawn((
                    MatchEntity,
                    crate::bushes::Bush { yaw: prop.yaw, radius: 0.75 * prop.scale, shake: 0.0, heard: 0.0 },
                    Transform::from_translation(prop.pos).with_rotation(Quat::from_rotation_y(prop.yaw)),
                    Visibility::Inherited,
                ))
                .id();
            art.spawn(&mut commands, entity, &baked);
            continue;
        }
        let key = ((prop.pos.x / PROP_REGION).floor() as i32, (prop.pos.z / PROP_REGION).floor() as i32);
        let (lit, unlit) = regions.entry(key).or_default();
        // Boxy props stay axis-aligned so they match their collision boxes.
        let yaw = if matches!(prop.kind, PropKind::Palm | PropKind::Rock | PropKind::Bush | PropKind::Lamp) { prop.yaw } else { 0.0 };
        art.add_model(lit, unlit, &prop_model(prop), Transform::from_translation(prop.pos).with_rotation(Quat::from_rotation_y(yaw)));
    }
    // Architecture: every wall, floor, stair and trim, merged with the props of its region.
    for block in &map.blocks {
        let at = block.aabb.center;
        let key = ((at.x / PROP_REGION).floor() as i32, (at.z / PROP_REGION).floor() as i32);
        let (lit, unlit) = regions.entry(key).or_default();
        let part = Part { shape: Shape::Cube, paint: block.paint, pos: at, size: block.aabb.half * 2.0, rot: Quat::IDENTITY, anim: Anim::None };
        art.add_part(if block.paint.unlit() { unlit } else { lit }, &part, Transform::IDENTITY);
    }
    // Foam streaks on the water give the sea some life for free.
    let mut rng = Rng::new(rules.recipe.seed ^ 0xf0a);
    let mut foam = MeshBuilder::default();
    for i in 0..260 {
        let (x, z) = (rng.range(-1.0, 1.0) * rules.recipe.half_extent, rng.range(-1.0, 1.0) * rules.recipe.half_extent);
        if map.height(x, z) > map.water - 0.1 {
            continue;
        }
        let part = Part {
            shape: Shape::Cube,
            paint: if i % 4 == 0 { Paint::White } else { Paint::Aether },
            pos: Vec3::new(x, map.water + 0.02, z),
            size: Vec3::new(rng.range(0.7, 4.0), 0.02, 0.13),
            rot: Quat::IDENTITY,
            anim: Anim::None,
        };
        art.add_part(&mut foam, &part, Transform::IDENTITY);
    }
    regions.entry((i32::MAX, 0)).or_default().1 = foam;
    for (_, (lit, unlit)) in regions {
        if !lit.is_empty() {
            commands.spawn((MatchEntity, Mesh3d(meshes.add(lit.build())), MeshMaterial3d(art.lit.clone()), Transform::IDENTITY));
        }
        if !unlit.is_empty() {
            commands.spawn((MatchEntity, Mesh3d(meshes.add(unlit.build())), MeshMaterial3d(art.unlit.clone()), Transform::IDENTITY, NotShadowCaster));
        }
    }

    // The objective.
    if rules.objective_hp > 0.0 {
        let root = commands.spawn((MatchEntity, Transform::from_translation(map.ground(map.core.x, map.core.y)))).id();
        let baked = art.bake(&content.objective_model, &mut meshes);
        art.spawn(&mut commands, root, &baked);
    }

    // Sky and lights.
    let size = 128;
    commands.insert_resource(Sky {
        day: images.add(cubemap(sky_cubemap(&look, false, size), size)),
        night: images.add(cubemap(sky_cubemap(&look, true, size), size)),
        fog: Vec3::from(look.fog),
        water: Vec3::from(look.water),
        water_material,
    });
    commands.insert_resource(AmbientLight { color: Color::srgb(0.71, 0.82, 1.0), brightness: 420.0, ..default() });
    commands.spawn((
        MatchEntity,
        Sun,
        DirectionalLight { color: Color::srgb(1.0, 0.76, 0.54), illuminance: 6500.0, shadows_enabled: settings.shadows, ..default() },
        // Two short cascades: crisp shadows near the player, nothing wasted on the horizon.
        CascadeShadowConfigBuilder { num_cascades: 2, maximum_distance: 70.0, first_cascade_far_bound: 18.0, ..default() }.build(),
        Transform::from_xyz(-0.54, 0.6, -0.81).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// Blends lighting, fog, sky and sea between day and night from `MatchState::daylight`.
fn day_night(
    state: Res<MatchState>,
    rules: Res<Rules>,
    sky: Res<Sky>,
    settings: Res<Settings>,
    mut ambient: ResMut<AmbientLight>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut suns: Query<&mut DirectionalLight, With<Sun>>,
    mut cameras: Query<(&mut Skybox, &mut DistanceFog), With<WorldCamera>>,
    mut lamps: Query<&mut SpotLight, With<Flashlight>>,
    mut last: Local<(f32, Option<oz_core::worldgen::Weather>, bool)>,
    night_vision: Res<crate::optics::NightVision>,
    lamp: Res<crate::tactical::Lamp>,
) {
    let last = &mut *last;
    let nv_changed = last.2 != night_vision.on;
    last.2 = night_vision.on;
    let last = (&mut last.0, &mut last.1);
    let day = state.daylight.clamp(0.0, 1.0);
    // Weather: grey skies dim the sun and lift the ambient; fog closes in.
    let weather = state.weather;
    let _ = &rules;
    let (sun_scale, fog_scale) = match weather {
        oz_core::worldgen::Weather::Clear => (1.0, 1.0),
        oz_core::worldgen::Weather::Overcast => (0.3, 0.75),
        oz_core::worldgen::Weather::Rain => (0.12, 0.5),
        oz_core::worldgen::Weather::Fog => (0.35, 0.28),
    };
    if (day - *last.0).abs() < 0.002 && *last.1 == Some(state.weather) && !nv_changed && !settings.is_changed() && !sky.is_added() && !lamp.is_changed() {
        return;
    }
    *last.0 = day;
    *last.1 = Some(state.weather);
    let mix = |night: Vec3, noon: Vec3| night.lerp(noon, day);

    if let Ok(mut sun) = suns.single_mut() {
        let color = mix(Vec3::new(0.45, 0.58, 1.0), Vec3::new(1.0, 0.76, 0.54));
        sun.color = Color::srgb(color.x, color.y, color.z);
        sun.illuminance = (500.0 + 6000.0 * day) * sun_scale;
        sun.shadows_enabled = settings.shadows;
    }
    let ambient_color = mix(Vec3::new(0.35, 0.45, 0.85), if sun_scale < 1.0 { Vec3::new(0.72, 0.76, 0.82) } else { Vec3::new(0.71, 0.82, 1.0) });
    ambient.color = Color::srgb(ambient_color.x, ambient_color.y, ambient_color.z);
    ambient.brightness = (110.0 + 310.0 * day) * (0.75 + sun_scale * 0.25);
    if night_vision.on {
        // The visor amplifies every scrap of light, and greens it.
        ambient.color = Color::srgb(0.5, 1.0, 0.55);
        ambient.brightness = (ambient.brightness * 6.0).max(1400.0);
    }

    if let Ok((mut skybox, mut fog)) = cameras.single_mut() {
        // Cross-fade through black: dim the day sky, swap, brighten the night sky.
        let (image, brightness) = if day >= 0.5 { (&sky.day, 1000.0 * (day - 0.5) * 2.0) } else { (&sky.night, 900.0 * (0.5 - day) * 2.0) };
        if skybox.image != *image {
            skybox.image = image.clone();
        }
        skybox.brightness = brightness.max(30.0) * (0.35 + 0.65 * sun_scale);
        // Grey weather greys the haze.
        let haze = if sun_scale < 1.0 { sky.fog.lerp(Vec3::new(0.52, 0.55, 0.58), 0.7) } else { sky.fog };
        let fog_color = mix(Vec3::new(0.02, 0.035, 0.07), haze * (0.6 + 0.4 * sun_scale));
        fog.color = Color::srgb(fog_color.x, fog_color.y, fog_color.z);
        fog.falloff = FogFalloff::Linear { start: (35.0 + 25.0 * day) * fog_scale, end: (120.0 + 110.0 * day) * fog_scale.max(0.4) };
    }
    if let Some(material) = materials.get_mut(&sky.water_material) {
        let water = mix(Vec3::new(0.02, 0.06, 0.10), sky.water);
        material.base_color = Color::srgb(water.x, water.y, water.z);
    }
    let lamp_on = lamp.on;
    if let Ok(mut lamp) = lamps.single_mut() {
        // Switched with the d-pad (hold down) / H; when on it also lights dark rooms by day.
        lamp.intensity = if lamp_on { 1_800_000.0 * (1.0 - day) } else { 0.0 };
    }
}
