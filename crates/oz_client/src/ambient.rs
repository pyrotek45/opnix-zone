//! The living parts of a map that are only for the eyes: grass, the
//! fountain's spray, flags rippling on the rooftops, clouds drifting across
//! the sky, and the weather - rain streaking past the camera, grey light for
//! overcast days, thick fog.

use crate::{
    ClientSet,
    art::{Art, MeshBuilder},
    view::WorldCamera,
};
use bevy::{pbr::NotShadowCaster, prelude::*};
use oz_core::{model::Part, worldgen::Weather};
use oz_sim::prelude::*;

const RAIN_DROPS: usize = 700;
const SPRAY: usize = 70;

#[derive(Component)]
struct Droplet {
    velocity: Vec3,
    life: f32,
}

#[derive(Component)]
struct Flag {
    phase: f32,
}

#[derive(Component)]
struct Cloud {
    speed: f32,
}

#[derive(Component)]
struct RainDrop;

pub struct AmbientPlugin;

impl Plugin for AmbientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(MatchPhase::Active), dress_world.after(crate::scene::BuildScene))
            .add_systems(Update, (fountain, flags, clouds, rain).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

fn dress_world(mut commands: Commands, map: Res<Map>, rules: Res<Rules>, art: Res<Art>, mut meshes: ResMut<Assets<Mesh>>) {
    let mut rng = Rng::new(rules.recipe.seed ^ 0x9a55);
    // Grass: a few tilted blades per tuft, all in one mesh.
    let mut grass = MeshBuilder::default();
    for tuft in &map.grass {
        for _ in 0..3 {
            let lean = Quat::from_euler(EulerRot::YXZ, rng.range(0.0, std::f32::consts::TAU), rng.range(-0.35, 0.35), rng.range(-0.35, 0.35));
            let height = rng.range(0.18, 0.42);
            let at = *tuft + Vec3::new(rng.range(-0.2, 0.2), height * 0.5, rng.range(-0.2, 0.2));
            let paint = if rng.chance(0.5) { Paint::Leaf } else { Paint::LeafLight };
            let part = Part { shape: Shape::Cube, paint, pos: at, size: Vec3::new(0.035, height, 0.035), rot: lean, anim: Anim::None };
            art.add_part(&mut grass, &part, Transform::IDENTITY);
        }
    }
    if !grass.is_empty() {
        commands.spawn((MatchEntity, Mesh3d(meshes.add(grass.build())), MeshMaterial3d(art.lit.clone()), Transform::IDENTITY, NotShadowCaster));
    }

    // Flags on their poles.
    for (i, top) in map.flags.iter().enumerate() {
        let paint = [Paint::Red, Paint::Gold, Paint::Teal, Paint::Coral][i % 4];
        commands
            .spawn((MatchEntity, Flag { phase: i as f32 * 1.7 }, Transform::from_translation(*top), Visibility::Inherited))
            .with_children(|pivot| {
                pivot.spawn((Mesh3d(art.primitive(Shape::Cube)), MeshMaterial3d(art.paint(paint)), Transform::from_xyz(0.5, -0.25, 0.0).with_scale(Vec3::new(1.0, 0.55, 0.03)), NotShadowCaster));
            });
    }

    // Clouds: big flat blocks high up, drifting with the wind.
    let overcast = matches!(rules.recipe.weather, Weather::Overcast | Weather::Rain);
    let count = if overcast { 34 } else { 16 };
    for _ in 0..count {
        let at = Vec3::new(rng.range(-220.0, 220.0), rng.range(70.0, 95.0), rng.range(-220.0, 220.0));
        let size = Vec3::new(rng.range(18.0, 45.0), rng.range(3.0, 7.0), rng.range(12.0, 30.0));
        let paint = if overcast { if rng.chance(0.5) { Paint::Iron } else { Paint::Steel } } else if rng.chance(0.6) { Paint::White } else { Paint::Bone };
        commands.spawn((MatchEntity, Cloud { speed: rng.range(1.5, 3.5) }, Mesh3d(art.primitive(Shape::Cube)), MeshMaterial3d(art.paint(paint)), Transform::from_translation(at).with_scale(size), NotShadowCaster));
    }

    // Rain: a cloud of streaks that follows the camera around.
    if rules.recipe.weather == Weather::Rain {
        for _ in 0..RAIN_DROPS {
            let at = Vec3::new(rng.range(-18.0, 18.0), rng.range(0.0, 22.0), rng.range(-18.0, 18.0));
            commands.spawn((MatchEntity, RainDrop, Mesh3d(art.primitive(Shape::Cube)), MeshMaterial3d(art.paint(Paint::Bone)), Transform::from_translation(at).with_scale(Vec3::new(0.03, 0.7, 0.03)), NotShadowCaster));
        }
    }
}

/// The fountain throws up water that falls back into the bowl.
fn fountain(time: Res<Time>, map: Res<Map>, art: Res<Art>, mut commands: Commands, mut drops: Query<(Entity, &mut Transform, &mut Droplet)>, mut seed: Local<u32>, mut timer: Local<f32>) {
    let Some(top) = map.fountain else { return };
    let dt = time.delta_secs().min(0.05);
    let mut alive = 0;
    for (entity, mut transform, mut drop) in &mut drops {
        drop.life -= dt;
        drop.velocity.y -= 9.8 * dt;
        transform.translation += drop.velocity * dt;
        if drop.life <= 0.0 || transform.translation.y < top.y - 2.6 {
            commands.entity(entity).despawn();
        } else {
            alive += 1;
        }
    }
    *timer -= dt;
    while *timer <= 0.0 && alive < SPRAY {
        *timer += 0.025;
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let angle = (*seed >> 8) as f32 / (1u32 << 24) as f32 * std::f32::consts::TAU;
        let out = 0.6 + ((*seed >> 4) & 0xff) as f32 / 255.0 * 0.8;
        let velocity = Vec3::new(angle.cos() * out, 3.4, angle.sin() * out);
        let paint = if *seed % 3 == 0 { Paint::White } else { Paint::Aether };
        commands.spawn((MatchEntity, Droplet { velocity, life: 1.6 }, Mesh3d(art.primitive(Shape::Shard)), MeshMaterial3d(art.paint(paint)), Transform::from_translation(top).with_scale(Vec3::splat(0.07)), NotShadowCaster));
        alive += 1;
    }
}

/// Flags flap: a swinging heading and a rippling width.
fn flags(time: Res<Time>, mut flags: Query<(&mut Transform, &Flag)>) {
    let t = time.elapsed_secs();
    for (mut transform, flag) in &mut flags {
        let p = t * 3.1 + flag.phase;
        transform.rotation = Quat::from_rotation_y(0.6 + 0.25 * p.sin() + 0.1 * (p * 2.3).sin());
        transform.scale = Vec3::new(1.0 - 0.12 * (p * 1.7).sin().abs(), 1.0, 1.0);
    }
}

fn clouds(time: Res<Time>, mut clouds: Query<(&mut Transform, &Cloud)>) {
    let dt = time.delta_secs();
    for (mut transform, cloud) in &mut clouds {
        transform.translation.x += cloud.speed * dt;
        if transform.translation.x > 230.0 {
            transform.translation.x -= 460.0;
        }
    }
}

/// Rain streaks fall around the camera and wrap back to the top.
fn rain(time: Res<Time>, camera: Query<&Transform, (With<WorldCamera>, Without<RainDrop>)>, mut drops: Query<&mut Transform, With<RainDrop>>) {
    let Ok(camera) = camera.single() else { return };
    let dt = time.delta_secs().min(0.05);
    let wind = Vec3::new(1.5, -19.0, 0.6);
    for mut transform in &mut drops {
        transform.translation += wind * dt;
        // Keep every drop inside a box around the camera.
        let local = transform.translation - camera.translation;
        if local.y < -4.0 {
            transform.translation.y += 22.0;
        }
        for axis in [0usize, 2] {
            if local[axis] > 18.0 {
                transform.translation[axis] -= 36.0;
            } else if local[axis] < -18.0 {
                transform.translation[axis] += 36.0;
            }
        }
    }
}
