//! The showroom: a small lit stage, far above the world, that menus use as
//! a backdrop. It shows your robot in the main menu, your squad in the lobby,
//! your class in the loadout screen, a rotating gun in the gunsmith, and a
//! lineup of every pose for model previews.
//!
//! It renders to its own low-resolution texture behind the UI. While it is
//! up, the world camera is switched off, so menus cost no more than the game.

use crate::{
    ClientSet,
    art::{Art, Baked},
    avatar::{Pose, RobotParts, face, spawn_robot},
    input::Controls,
    retro::Retro,
    settings::Settings,
    view::WorldCamera,
};
use bevy::{
    asset::RenderAssetUsages,
    core_pipeline::tonemapping::Tonemapping,
    image::ImageSampler,
    input::mouse::AccumulatedMouseMotion,
    pbr::NotShadowCaster,
    prelude::*,
    render::{
        camera::RenderTarget,
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
    },
    window::PrimaryWindow,
};
use oz_core::arsenal::{SLOT_COUNT, assemble};
use oz_sim::prelude::*;

/// Where the stage lives: far outside any map and beyond the world camera's far plane.
const STAGE: Vec3 = Vec3::new(0.0, 3000.0, 0.0);

/// What the stage is showing.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub enum Stage {
    #[default]
    Hidden,
    /// One robot, standing to the right of a left-hand menu panel. `weapon` uses the match catalog.
    Operator { team: u8, weapon: Option<(u8, [u8; SLOT_COUNT])> },
    /// A row of robots (lobby). `me` is drawn in the middle.
    Squad { team: u8, count: u8 },
    /// One gun, centred and turning.
    Gun { weapon: u8, parts: [u8; SLOT_COUNT] },
    /// Every pose side by side, for model previews.
    Lineup,
}

#[derive(Component)]
struct ShowCamera;

#[derive(Component)]
struct ShowImage;

#[derive(Component)]
struct Subject;

#[derive(Component)]
struct Turntable;

/// Robots on the stage animate on their own.
#[derive(Component)]
struct Idle {
    seed: f32,
    base: Pose,
}

#[derive(Resource)]
struct ShowTarget(Handle<Image>);

/// Current turntable angle and the stage last built.
#[derive(Resource, Default)]
pub struct Turn {
    pub yaw: f32,
    built: Option<Stage>,
}

pub struct ShowroomPlugin;

impl Plugin for ShowroomPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stage>()
            .init_resource::<Turn>()
            .add_systems(Startup, setup_stage)
            .add_systems(Update, (resize, rebuild, spin, idle_robots, toggle).chain().in_set(ClientSet::Present).before(crate::avatar::animate_rigs));
    }
}

fn image(width: u32, height: u32) -> Image {
    let size = Extent3d { width: width.max(16), height: height.max(16), depth_or_array_layers: 1 };
    let mut image = Image::new_fill(size, TextureDimension::D2, &[0, 0, 0, 255], TextureFormat::bevy_default(), RenderAssetUsages::default());
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    image.sampler = ImageSampler::nearest();
    image
}

fn setup_stage(mut commands: Commands, mut images: ResMut<Assets<Image>>, art: Res<Art>, mut meshes: ResMut<Assets<Mesh>>) {
    let target = images.add(image(640, 360));
    commands.spawn((
        ShowCamera,
        Camera3d::default(),
        Camera { target: RenderTarget::Image(target.clone().into()), order: -1, is_active: false, clear_color: ClearColorConfig::Custom(Color::srgb(0.02, 0.03, 0.05)), ..default() },
        Projection::Perspective(PerspectiveProjection { fov: 38f32.to_radians(), near: 0.05, far: 40.0, ..default() }),
        Msaa::Off,
        Tonemapping::Reinhard,
        Retro::default(),
        Transform::from_translation(STAGE + Vec3::new(0.0, 1.3, 6.0)).looking_at(STAGE + Vec3::Y, Vec3::Y),
    ));
    commands.spawn((
        ShowImage,
        ImageNode::new(target.clone()),
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        GlobalZIndex(-90),
        Visibility::Hidden,
    ));

    // The set: a dark studio floor, a curved back wall with light strips, and pedestals.
    let mut lit = crate::art::MeshBuilder::default();
    let mut unlit = crate::art::MeshBuilder::default();
    let mut set = BlockModel::new()
        .cylinder(Paint::Steel, [0.0, -0.05, 0.0], [26.0, 0.1, 26.0])
        .cylinder(Paint::Dark, [0.0, -0.04, 0.0], [9.0, 0.1, 9.0]);
    for i in 0..15 {
        let angle = (i as f32 - 7.0) * 0.17;
        let at = Vec3::new(angle.sin() * 9.0, 0.0, -angle.cos() * 9.0 + 1.5);
        set = set
            .cube(if i % 2 == 0 { Paint::Steel } else { Paint::Dark }, [at.x, 3.0, at.z], [1.6, 6.0, 0.3])
            .rotated([0.0, -angle.to_degrees(), 0.0])
            .cube(if i % 3 == 0 { Paint::Glow } else { Paint::Blue }, [at.x, 1.2 + (i % 3) as f32 * 1.4, at.z + 0.2], [1.3, 0.05, 0.05])
            .rotated([0.0, -angle.to_degrees(), 0.0]);
    }
    for x in [-3.9, -2.6, -1.3, 0.0, 1.3, 2.6, 3.9] {
        set = set.cylinder(Paint::Dark, [x, 0.05, 0.0], [1.0, 0.1, 1.0]).cylinder(Paint::Aether, [x, 0.105, 0.0], [1.05, 0.01, 1.05]).cylinder(Paint::Dark, [x, 0.112, 0.0], [0.9, 0.012, 0.9]);
    }
    art.add_model(&mut lit, &mut unlit, &set, Transform::from_translation(STAGE));
    commands.spawn((Mesh3d(meshes.add(lit.build())), MeshMaterial3d(art.lit.clone()), Transform::IDENTITY));
    commands.spawn((Mesh3d(meshes.add(unlit.build())), MeshMaterial3d(art.unlit.clone()), Transform::IDENTITY, NotShadowCaster));

    // Key, fill and rim lights. Point and spot lights only reach the stage, never the world.
    for (at, color, intensity) in [
        (Vec3::new(-3.0, 4.0, 4.0), Color::srgb(1.0, 0.86, 0.7), 2_600_000.0),
        (Vec3::new(3.5, 2.5, 3.0), Color::srgb(0.55, 0.75, 1.0), 1_200_000.0),
        (Vec3::new(0.0, 3.5, -3.0), Color::srgb(0.4, 1.0, 0.9), 1_400_000.0),
    ] {
        commands.spawn((PointLight { color, intensity, range: 18.0, shadows_enabled: false, ..default() }, Transform::from_translation(STAGE + at)));
    }
    commands.spawn((
        SpotLight { color: Color::srgb(1.0, 0.95, 0.85), intensity: 3_000_000.0, range: 16.0, outer_angle: 0.6, inner_angle: 0.35, shadows_enabled: true, ..default() },
        Transform::from_translation(STAGE + Vec3::new(1.0, 6.0, 2.5)).looking_at(STAGE, Vec3::Y),
    ));
    commands.insert_resource(ShowTarget(target));
}

fn resize(settings: Res<Settings>, target: Res<ShowTarget>, windows: Query<&Window, With<PrimaryWindow>>, mut images: ResMut<Assets<Image>>) {
    let Ok(window) = windows.single() else { return };
    // Menus are a little crisper than the game: half the pixel size, at least 2.
    let scale = (settings.pixel_size / 2).max(2);
    let wanted = UVec2::new((window.physical_width() / scale).max(16), (window.physical_height() / scale).max(16));
    if images.get(&target.0).map(|i| i.size()) != Some(wanted)
        && let Some(image) = images.get_mut(&target.0) {
            image.resize(Extent3d { width: wanted.x, height: wanted.y, depth_or_array_layers: 1 });
        }
}

/// Centre and size of a model's parts, for framing.
fn bounds(model: &BlockModel) -> (Vec3, Vec3) {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for part in &model.parts {
        lo = lo.min(part.pos - part.size * 0.5);
        hi = hi.max(part.pos + part.size * 0.5);
    }
    if lo.x > hi.x { (Vec3::ZERO, Vec3::ONE) } else { ((lo + hi) * 0.5, hi - lo) }
}

fn kit_gun(art: &Art, meshes: &mut Assets<Mesh>) -> Baked {
    let guns = oz_core::kit::standard_guns();
    let def = &guns[oz_core::kit::gun::CARBINE];
    art.bake(&assemble(def, &def.defaults), meshes)
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    stage: Res<Stage>,
    mut turn: ResMut<Turn>,
    art: Res<Art>,
    parts: Res<RobotParts>,
    content: Option<Res<Content>>,
    mut meshes: ResMut<Assets<Mesh>>,
    subjects: Query<Entity, With<Subject>>,
    mut camera: Query<&mut Transform, With<ShowCamera>>,
) {
    if turn.built.as_ref() == Some(&*stage) {
        return;
    }
    // Gun previews swap parts constantly; keep the turntable angle when only parts change.
    let same_gun = matches!((&turn.built, &*stage), (Some(Stage::Gun { weapon: a, .. }), Stage::Gun { weapon: b, .. }) if a == b);
    turn.built = Some(stage.clone());
    if !same_gun {
        turn.yaw = 0.6;
    }
    for entity in &subjects {
        commands.entity(entity).despawn();
    }
    let Ok(mut camera) = camera.single_mut() else { return };
    let gun_for = |weapon: Option<(u8, [u8; SLOT_COUNT])>, meshes: &mut Assets<Mesh>| -> Baked {
        match (weapon, content.as_deref()) {
            (Some((index, equipped)), Some(content)) => match content.weapons.get(index as usize) {
                Some(def) => art.bake(&assemble(def, &equipped), meshes),
                None => kit_gun(&art, meshes),
            },
            _ => kit_gun(&art, meshes),
        }
    };
    let robot = |commands: &mut Commands, at: Vec3, yaw: f32, team: u8, gun: &Baked, pose: Pose, seed: f32| {
        // Models face -Z; the camera looks from +Z, so turn them round to face it.
        let root = commands.spawn((Subject, Transform::from_translation(STAGE + at + Vec3::Y * 0.11).with_rotation(Quat::from_rotation_y(yaw + std::f32::consts::PI)))).id();
        let rig = spawn_robot(commands, &art, &parts, root, team, Some(gun));
        commands.entity(root).insert((rig, pose, Idle { seed, base: pose }));
        root
    };

    match &*stage {
        Stage::Hidden => {}
        Stage::Operator { team, weapon } => {
            let gun = gun_for(*weapon, &mut meshes);
            let root = robot(&mut commands, Vec3::ZERO, 0.35, *team, &gun, Pose::default(), 0.0);
            commands.entity(root).insert(Turntable);
            *camera = Transform::from_translation(STAGE + Vec3::new(-1.35, 1.25, 4.4)).looking_at(STAGE + Vec3::new(-1.35, 1.0, 0.0), Vec3::Y);
        }
        Stage::Squad { team, count } => {
            let gun = gun_for(None, &mut meshes);
            let slots = [0.0, -1.3, 1.3, -2.6, 2.6, -3.9];
            for (i, x) in slots.iter().take((*count as usize).clamp(1, 6)).enumerate() {
                let pose = Pose { stance: if i % 3 == 2 { stance::CROUCH } else { stance::STAND }, ..default() };
                robot(&mut commands, Vec3::new(*x, 0.0, (i as f32 * 0.37).sin() * 0.2), (*x * 0.08) + 0.1, *team, &gun, pose, i as f32 * 1.7);
            }
            *camera = Transform::from_translation(STAGE + Vec3::new(0.0, 1.5, 7.4)).looking_at(STAGE + Vec3::new(0.0, 0.95, 0.0), Vec3::Y);
        }
        Stage::Gun { weapon, parts: equipped } => {
            let model = content.as_deref().and_then(|c| c.weapons.get(*weapon as usize)).map_or_else(BlockModel::new, |def| assemble(def, equipped));
            let (center, size) = bounds(&model);
            let scale = 1.5 / size.z.max(size.y).max(0.3);
            let baked = art.bake(&model, &mut meshes);
            let pivot = commands.spawn((Subject, Turntable, Transform::from_translation(STAGE + Vec3::Y * 1.2), Visibility::Inherited)).id();
            let holder = commands.spawn((Transform::from_translation(-center * scale).with_scale(Vec3::splat(scale)), Visibility::Inherited, ChildOf(pivot))).id();
            art.spawn(&mut commands, holder, &baked);
            *camera = Transform::from_translation(STAGE + Vec3::new(0.0, 1.45, 2.6)).looking_at(STAGE + Vec3::new(0.0, 1.2, 0.0), Vec3::Y);
        }
        Stage::Lineup => {
            let gun = gun_for(None, &mut meshes);
            let poses = [
                (Pose { face: Some(face::SMILE), ..default() }, 0.3),
                (Pose { speed: 6.0, face: Some(face::BLINK), ..default() }, 0.9),
                (Pose { stance: stance::CROUCH, aiming: true, ..default() }, 0.2),
                (Pose { stance: stance::PRONE, aiming: true, pitch: 0.05, ..default() }, 0.9),
                (Pose { pitch: 0.7, aiming: true, ..default() }, -0.3),
                (Pose { reloading: true, face: Some(face::HURT), ..default() }, 0.4),
                (Pose { dead: true, ..default() }, 0.6),
            ];
            for (i, (pose, yaw)) in poses.into_iter().enumerate() {
                robot(&mut commands, Vec3::new(-3.9 + i as f32 * 1.3, 0.0, 0.0), yaw, (i % 2) as u8, &gun, pose, i as f32);
            }
            *camera = Transform::from_translation(STAGE + Vec3::new(0.0, 1.9, 8.6)).looking_at(STAGE + Vec3::new(0.0, 0.8, 0.0), Vec3::Y);
        }
    }
}

/// Turns the subject: slowly on its own, or by dragging / the right stick.
fn spin(
    time: Res<Time<Real>>,
    stage: Res<Stage>,
    mut turn: ResMut<Turn>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    controls: Res<Controls>,
    gamepads: Query<&Gamepad>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut tables: Query<&mut Transform, (With<Turntable>, With<Subject>)>,
) {
    let dt = time.delta_secs();
    let centre_drag = windows.single().ok().and_then(|w| w.cursor_position().map(|c| (c.x / w.width() - 0.5).abs() < 0.2)).unwrap_or(false);
    let stick: f32 = gamepads.iter().map(|pad| pad.right_stick().x).filter(|x| x.abs() > 0.2).sum();
    if mouse.pressed(MouseButton::Left) && centre_drag && matches!(*stage, Stage::Gun { .. }) {
        turn.yaw += motion.delta.x * 0.01;
    } else if stick != 0.0 {
        turn.yaw += stick * dt * 2.5;
    } else if !controls.gamepad || stick == 0.0 {
        turn.yaw += dt * if matches!(*stage, Stage::Gun { .. }) { 0.35 } else { 0.12 };
    }
    for mut transform in &mut tables {
        transform.rotation = if matches!(*stage, Stage::Gun { .. }) {
            Quat::from_rotation_y(turn.yaw) * Quat::from_rotation_x(0.12 * (turn.yaw * 0.7).sin())
        } else {
            Quat::from_rotation_y(std::f32::consts::PI + 0.35 + 0.35 * (turn.yaw).sin())
        };
    }
}

/// Showroom robots breathe, glance around and blink.
fn idle_robots(time: Res<Time<Real>>, mut robots: Query<(&mut Pose, &Idle)>) {
    let t = time.elapsed_secs();
    for (mut pose, idle) in &mut robots {
        let base = idle.base;
        pose.pitch = base.pitch + 0.12 * (t * 0.6 + idle.seed).sin();
        pose.speed = base.speed;
        if base.face.is_none() {
            let cycle = (t * 0.35 + idle.seed).rem_euclid(6.0);
            pose.face = Some(if cycle > 5.85 { face::BLINK } else if cycle < 1.0 && idle.seed > 0.0 { face::ANGRY } else { face::SMILE });
        }
    }
}

/// Shows the stage and parks the world camera while it is up.
fn toggle(stage: Res<Stage>, mut show: Query<&mut Camera, (With<ShowCamera>, Without<WorldCamera>)>, mut world: Query<&mut Camera, With<WorldCamera>>, mut image: Query<&mut Visibility, With<ShowImage>>) {
    let up = *stage != Stage::Hidden;
    if let Ok(mut camera) = show.single_mut()
        && camera.is_active != up {
            camera.is_active = up;
        }
    for mut camera in &mut world {
        if camera.is_active == up {
            camera.is_active = !up;
        }
    }
    if let Ok(mut visibility) = image.single_mut() {
        let wanted = if up { Visibility::Visible } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}
