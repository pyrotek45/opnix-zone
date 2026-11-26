//! The first-person view: cameras, the low-resolution render target and the viewmodel.
//!
//! The 3D world is rendered into a small texture (window size divided by the
//! `pixel_size` setting) and blown up with nearest-neighbour filtering. That
//! is where both the chunky look and most of the frame rate come from: at
//! pixel size 3 the GPU shades one ninth of the pixels.

use crate::{
    ClientSet,
    art::Art,
    retro::Retro,
    scene::Sky,
    settings::Settings,
};
use bevy::{
    asset::RenderAssetUsages,
    core_pipeline::{Skybox, tonemapping::Tonemapping},
    image::ImageSampler,
    pbr::{NotShadowCaster, NotShadowReceiver},
    prelude::*,
    render::{
        camera::RenderTarget,
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
    },
    window::PrimaryWindow,
};
use oz_core::arsenal::{part_model, resolve};
use oz_sim::prelude::*;

pub const BASE_FOV: f32 = 70.0;
const GUN_POS: Vec3 = Vec3::new(0.21, -0.19, -0.50);
/// The viewmodel is drawn smaller than life so it frames the view instead of filling it.
const GUN_SCALE: f32 = 0.62;

/// The 3D camera rendering the match into the low-resolution target.
#[derive(Component)]
pub struct WorldCamera;

/// The UI camera; it also presents the world texture.
#[derive(Component)]
pub struct UiCamera;

#[derive(Component)]
struct WorldImage;

#[derive(Component)]
struct GunRoot;

#[derive(Component)]
struct MuzzleFlash;

#[derive(Component)]
pub struct Flashlight;

#[derive(Component)]
struct MuzzleLight;

#[derive(Resource)]
pub struct WorldTarget(pub Handle<Image>);

/// Transient first-person feel: recoil, aim blend, damage flash.
#[derive(Resource, Default)]
pub struct ViewState {
    pub ads: f32,
    pub recoil: f32,
    pub flash: f32,
    pub bob: f32,
    pub hurt: f32,
    pub hit: f32,
    pub kill: bool,
    /// Camera shake energy; explosions, hits and recoil add to it.
    pub shake: f32,
    /// Weapon lag behind the look direction.
    sway: Vec2,
    land: f32,
    swap: f32,
    sprint: f32,
    kick: f32,
    was_grounded: bool,
    fall_speed: f32,
    step: i32,
    last_weapon: Option<u8>,
    last_shots: u32,
    /// (weapon, equipped parts) currently shown in the hands.
    shown: Option<(u8, [u8; 5])>,
}

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewState>()
            .insert_resource(ClearColor(Color::srgb(0.02, 0.03, 0.05)))
            .add_systems(Startup, setup_presentation)
            .add_systems(Update, resize_target)
            .add_systems(Update, (follow_player, viewmodel).chain().in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

fn target_image(width: u32, height: u32) -> Image {
    let size = Extent3d { width: width.max(16), height: height.max(16), depth_or_array_layers: 1 };
    let mut image = Image::new_fill(size, TextureDimension::D2, &[0, 0, 0, 255], TextureFormat::bevy_default(), RenderAssetUsages::default());
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    image.sampler = ImageSampler::nearest();
    image
}

fn setup_presentation(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let handle = images.add(target_image(426, 240));
    commands.spawn((UiCamera, Camera2d, Camera { order: 1, ..default() }, Msaa::Off));
    commands.spawn((
        WorldImage,
        ImageNode::new(handle.clone()),
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        GlobalZIndex(-100),
        Visibility::Hidden,
    ));
    commands.insert_resource(WorldTarget(handle));
}

/// Keeps the render target at window size / pixel size.
fn resize_target(
    settings: Res<Settings>,
    target: Res<WorldTarget>,
    phase: Res<State<MatchPhase>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut view: Query<&mut Visibility, With<WorldImage>>,
) {
    if let Ok(mut visibility) = view.single_mut() {
        let wanted = if *phase.get() == MatchPhase::Active { Visibility::Visible } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    let Ok(window) = windows.single() else { return };
    let scale = settings.pixel_size.max(1);
    let (width, height) = ((window.physical_width() / scale).max(16), (window.physical_height() / scale).max(16));
    let current = images.get(&target.0).map(|image| image.size());
    if current != Some(UVec2::new(width, height)) {
        if let Some(image) = images.get_mut(&target.0) {
            image.resize(Extent3d { width, height, depth_or_array_layers: 1 });
        }
    }
}

/// Spawned by `scene` once the sky exists.
pub fn spawn_camera(mut commands: Commands, target: Res<WorldTarget>, sky: Res<Sky>, art: Res<Art>, mut view: ResMut<ViewState>) {
    *view = ViewState::default();
    commands
        .spawn((
            MatchEntity,
            WorldCamera,
            Camera3d::default(),
            Camera { target: RenderTarget::Image(target.0.clone().into()), order: 0, ..default() },
            Projection::Perspective(PerspectiveProjection { fov: BASE_FOV.to_radians(), near: 0.05, far: 600.0, ..default() }),
            Msaa::Off,
            Tonemapping::Reinhard,
            Retro::default(),
            DistanceFog { color: Color::srgb(0.65, 0.68, 0.67), falloff: FogFalloff::Linear { start: 60.0, end: 230.0 }, ..default() },
            Skybox { image: sky.day.clone(), brightness: 1000.0, ..default() },
            Transform::from_xyz(0.0, 30.0, 0.0),
        ))
        .with_children(|camera| {
            camera.spawn((GunRoot, Transform::from_translation(GUN_POS).with_scale(Vec3::splat(GUN_SCALE)), Visibility::Inherited));
            camera.spawn((
                MuzzleFlash,
                Mesh3d(art.primitive(Shape::Shard)),
                MeshMaterial3d(art.paint(Paint::Spark)),
                Transform::from_scale(Vec3::splat(0.1)),
                Visibility::Hidden,
                NotShadowCaster,
                NotShadowReceiver,
            ));
            camera.spawn((MuzzleLight, PointLight { color: Color::srgb(1.0, 0.72, 0.3), intensity: 0.0, range: 9.0, shadows_enabled: false, ..default() }, Transform::from_xyz(0.2, -0.05, -1.0)));
            // Helmet lamp: fades in at night so the dark stays moody but playable.
            camera.spawn((
                Flashlight,
                SpotLight { color: Color::srgb(1.0, 0.93, 0.8), intensity: 0.0, range: 45.0, outer_angle: 0.5, inner_angle: 0.3, shadows_enabled: false, ..default() },
                // Mounted ahead of the hands so it never washes out the viewmodel.
                Transform::from_xyz(0.0, 0.1, -1.1),
            ));
        });
}

fn follow_player(
    time: Res<Time>,
    content: Res<Content>,
    controls: Res<crate::input::Controls>,
    mut sounds: EventWriter<crate::fx::PlaySfx>,
    mut lights: Query<&mut PointLight, With<MuzzleLight>>,
    mut view: ResMut<ViewState>,
    players: Query<(&Transform, &PlayerInput, &Arsenal, &Mover, &Health), With<LocalPlayer>>,
    mut cameras: Query<(&mut Transform, &mut Projection), (With<WorldCamera>, Without<LocalPlayer>)>,
) {
    let dt = time.delta_secs().min(0.05);
    let (Ok((player, input, arsenal, mover, health)), Ok((mut camera, mut projection))) = (players.single(), cameras.single_mut()) else { return };

    // Recoil kicks whenever the authoritative shot counter advances.
    let index = arsenal.current as usize;
    if arsenal.shots != view.last_shots {
        view.last_shots = arsenal.shots;
        if let Some(def) = content.weapons.get(index) {
            view.recoil = (view.recoil + def.recoil).min(2.5);
            view.flash = 0.05;
            view.shake += def.recoil * 0.1;
            view.kick = (view.kick + def.recoil * 0.9).min(2.5);
        }
    }
    if view.last_weapon != Some(arsenal.current) {
        if view.last_weapon.is_some() {
            view.swap = 1.0;
            sounds.write(crate::fx::PlaySfx { sfx: Sfx::Swap, at: None });
        }
        view.last_weapon = Some(arsenal.current);
    }
    // Landing: dip the camera and thump in proportion to the fall.
    if mover.grounded && !view.was_grounded && view.fall_speed < -4.5 {
        view.land = (-view.fall_speed / 9.0).min(1.3);
        sounds.write(crate::fx::PlaySfx { sfx: Sfx::Land, at: None });
    }
    view.was_grounded = mover.grounded;
    view.fall_speed = mover.vertical;
    view.land = (view.land - dt * 5.0).max(0.0);
    view.swap = (view.swap - dt * 5.0).max(0.0);
    view.kick = (view.kick - dt * 9.0 * view.kick.max(0.3)).max(0.0);
    view.shake = (view.shake - dt * 5.0 * view.shake.max(0.25)).max(0.0).min(2.5);
    view.sway = (view.sway + controls.look * 0.6).clamp_length_max(0.06) * (-dt * 12.0).exp();
    let sprinting = input.held(buttons::SPRINT) && mover.speed > 6.5;
    view.sprint += (sprinting as u8 as f32 - view.sprint) * (1.0 - (-dt * 8.0).exp());
    if let Ok(mut light) = lights.single_mut() {
        light.intensity = if view.flash > 0.0 { 90_000.0 } else { 0.0 };
    }
    view.recoil = (view.recoil - dt * 6.0 * view.recoil.max(0.4)).max(0.0);
    view.flash = (view.flash - dt).max(0.0);
    view.hurt = (view.hurt - dt * 1.6).max(0.0);
    view.hit = (view.hit - dt).max(0.0);
    let walking = (mover.speed / 5.4).min(1.6) * mover.grounded as u8 as f32;
    view.bob += dt * 9.5 * walking;
    // One footstep per half bob cycle.
    let step = (view.bob / std::f32::consts::PI) as i32;
    if step != view.step {
        view.step = step;
        if walking > 0.3 && health.alive() {
            sounds.write(crate::fx::PlaySfx { sfx: Sfx::Step, at: None });
        }
    }

    let aiming = input.held(buttons::AIM) && arsenal.reload <= 0.0 && health.alive();
    view.ads += (aiming as u8 as f32 - view.ads) * (1.0 - (-dt * 16.0).exp());

    let ads_fov = content.weapons.get(index).zip(arsenal.weapons.get(index)).map_or(BASE_FOV, |(def, state)| resolve(def, state, &content.attachments).ads_fov);
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.fov = (BASE_FOV + (ads_fov - BASE_FOV) * view.ads + 5.0 * view.sprint * (1.0 - view.ads) + view.kick * 0.9).to_radians();
    }

    let bob = Vec3::new(0.0, (view.bob * 2.0).sin() * 0.035 * walking * (1.0 - view.ads), 0.0);
    let (eye, roll) = if health.alive() { (EYE, 0.0) } else { (0.45, 0.5) };
    let t = time.elapsed_secs();
    let shake = Vec3::new((t * 61.0).sin(), (t * 53.0).cos(), (t * 47.0).sin()) * view.shake * 0.012;
    camera.translation = player.translation + Vec3::Y * (eye - view.land * 0.13) + bob;
    camera.rotation = Quat::from_euler(EulerRot::YXZ, input.yaw + shake.x, input.pitch + view.recoil * 0.012 + shake.y, roll + shake.z);
}

#[allow(clippy::too_many_arguments)]
fn viewmodel(
    time: Res<Time>,
    content: Res<Content>,
    art: Res<Art>,
    mut view: ResMut<ViewState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
    players: Query<(&Arsenal, &Health, &Mover), With<LocalPlayer>>,
    mut gun: Query<(Entity, &mut Transform, &mut Visibility), (With<GunRoot>, Without<MuzzleFlash>)>,
    mut flash: Query<(&mut Transform, &mut Visibility), (With<MuzzleFlash>, Without<GunRoot>)>,
) {
    let (Ok((arsenal, health, mover)), Ok((root, mut transform, mut visibility))) = (players.single(), gun.single_mut()) else { return };
    let index = arsenal.current as usize;
    let (Some(def), Some(state)) = (content.weapons.get(index), arsenal.weapons.get(index)) else { return };
    let stats = resolve(def, state, &content.attachments);

    // Rebuild the hands only when the weapon or its attachments change.
    let signature = (arsenal.current, state.equipped);
    if view.shown != Some(signature) {
        view.shown = Some(signature);
        let mut model = def.model.clone();
        for slot in 0..5 {
            model = model.merged(&part_model(slot, state.equipped[slot] as usize, def.length), [0.0; 3]);
        }
        for (slot, installed) in state.mods.iter().enumerate() {
            if let Some((kind, _)) = installed {
                model = model.cube(kind.paint(), [-0.11, 0.02 + 0.05 * slot as f32, -0.2], [0.03, 0.035, 0.12]);
            }
        }
        let baked = art.bake(&model, &mut meshes);
        commands.entity(root).despawn_related::<Children>();
        commands.entity(root).with_children(|parent| {
            for (mesh, material) in [(&baked.lit, &art.lit), (&baked.unlit, &art.unlit)] {
                if let Some(mesh) = mesh {
                    parent.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), NotShadowCaster));
                }
            }
        });
    }

    *visibility = if health.alive() { Visibility::Inherited } else { Visibility::Hidden };
    let t = time.elapsed_secs();
    let reload = if arsenal.reload > 0.0 { (arsenal.reload / stats.reload * std::f32::consts::PI).sin() } else { 0.0 };
    let walking = (mover.speed / 5.4).min(1.6) * (1.0 - view.ads * 0.94);
    // Line the sights (or the optic) up with the centre of the screen.
    let ads_pos = Vec3::new(0.0, -0.19 * GUN_SCALE, -0.40);
    transform.translation = GUN_POS.lerp(ads_pos, view.ads)
        + Vec3::new(view.bob.sin() * 0.018 * walking + view.sway.x * 0.5, (view.bob * 2.0).sin() * 0.016 * walking - reload * 0.2 - view.swap * 0.22 - view.land * 0.05 - view.sway.y * 0.5 + (t * 1.3).sin() * 0.003, view.recoil * 0.05);
    transform.rotation = Quat::from_rotation_x(view.recoil * 0.07 - reload * 0.65 - view.swap * 0.5) * Quat::from_rotation_z(reload * 0.26 + view.sway.x * 1.6);

    if let Ok((mut flash_transform, mut flash_visibility)) = flash.single_mut() {
        let lit = view.flash > 0.0 && health.alive() && def.mag > 0;
        *flash_visibility = if lit { Visibility::Inherited } else { Visibility::Hidden };
        flash_transform.translation = transform.translation + transform.rotation * (Vec3::new(0.0, 0.02, -stats.length - 0.08) * GUN_SCALE);
        flash_transform.scale = Vec3::new(0.06, 0.06, 0.15) * (0.7 + view.flash * 14.0);
        flash_transform.rotation = Quat::from_rotation_z(t * 31.0);
    }
}
