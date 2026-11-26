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
use oz_core::arsenal::{Action, GunClass, ReloadStyle, SLOT_COUNT, assemble, resolve};
use oz_sim::prelude::*;
use std::f32::consts::PI;

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
    /// Flashbang whiteout, in seconds (above 1 is fully white).
    pub blind: f32,
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
    shown: Option<(u8, [u8; SLOT_COUNT])>,
    /// Smoothed eye height (crouching and going prone glide).
    eye: f32,
    /// Smoothed peek offset out of cover.
    lean: Vec3,
    /// 0 = first person, 1 = over the shoulder (in cover).
    pub third: f32,
    /// A quick zoom-in on a kill, decaying from 1.
    pub punch: f32,
    /// The last hit marker was a headshot.
    pub head_hit: bool,
    /// Seconds left of the "limb off" marker flare.
    pub limb_hit: f32,
    /// 1 right after a shot, decaying over the weapon's cycle time (bolt / pump / slide animation).
    cycle: f32,
    /// Seconds left of the weapon inspect animation.
    pub inspect: f32,
    /// Scope overlay strength, 0..1 (0 = none, the HUD draws it).
    pub scoped: f32,
    pub scope_kind: u8,
}

/// The separately animated pieces of the viewmodel.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum GunGroup {
    Body,
    Mag,
    Slide,
    /// A round held in the hand during shell-by-shell reloads.
    Shell,
}

/// Label for the first-person camera; the spectator camera runs after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FollowPlayer;

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewState>()
            .init_resource::<ViewSubject>()
            .insert_resource(ClearColor(Color::srgb(0.02, 0.03, 0.05)))
            .add_systems(Startup, setup_presentation)
            .add_systems(Update, resize_target)
            .add_systems(Update, (follow_player, viewmodel).chain().in_set(FollowPlayer).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
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
    if current != Some(UVec2::new(width, height))
        && let Some(image) = images.get_mut(&target.0) {
            image.resize(Extent3d { width, height, depth_or_array_layers: 1 });
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

/// Whose eyes the first-person view uses: a spectated player, or (None) the local one.
#[derive(Resource, Default)]
pub struct ViewSubject(pub Option<Entity>);

fn follow_player(
    time: Res<Time>,
    content: Res<Content>,
    controls: Res<crate::input::Controls>,
    mut sounds: EventWriter<crate::fx::PlaySfx>,
    mut lights: Query<&mut PointLight, With<MuzzleLight>>,
    mut view: ResMut<ViewState>,
    (players, subject): (Query<(&Transform, &PlayerInput, &Arsenal, &Mover, &Health, &Player, Has<LocalPlayer>)>, Res<ViewSubject>),
    mut cameras: Query<(&mut Transform, &mut Projection), (With<WorldCamera>, Without<Player>)>,
    (role, prediction, settings, statics, cover): (Res<NetRole>, Res<crate::predict::Prediction>, Res<crate::settings::Settings>, Res<Statics>, Res<crate::cover::Cover>),
) {
    let dt = time.delta_secs().min(0.05);
    let pick = subject.0.and_then(|e| players.get(e).ok()).or_else(|| players.iter().find(|(.., local)| *local));
    let (Some((player, input, arsenal, mover, health, me, local)), Ok((mut camera, mut projection))) = (pick, cameras.single_mut()) else { return };

    // Recoil kicks whenever the shot counter advances: predicted locally when online.
    let index = held_weapon(input, arsenal) as usize;
    let shots = if *role == NetRole::Client && local { prediction.total } else { arsenal.shots };
    if shots != view.last_shots {
        view.last_shots = shots;
        if let Some(def) = content.weapons.get(index) {
            // Crouched and prone shooters are braced: less kick.
            let recoil = def.recoil * stance::recoil(input.stance);
            view.recoil = (view.recoil + recoil).min(2.5);
            view.flash = 0.05;
            view.shake += recoil * 0.1;
            view.kick = (view.kick + recoil * 0.9).min(2.5);
            view.cycle = 1.0;
        }
    }
    if controls.inspect && arsenal.reload <= 0.0 && view.ads < 0.1 {
        view.inspect = 2.2;
    }
    if input.held(buttons::FIRE) || input.held(buttons::AIM) || arsenal.reload > 0.0 {
        view.inspect = 0.0;
    }
    view.inspect = (view.inspect - dt).max(0.0);
    if view.last_weapon != Some(index as u8) {
        if view.last_weapon.is_some() {
            view.swap = 1.0;
            sounds.write(crate::fx::PlaySfx::new(Sfx::Swap, None));
        }
        view.last_weapon = Some(index as u8);
    }
    // Landing: dip the camera and thump in proportion to the fall.
    if mover.grounded && !view.was_grounded && view.fall_speed < -4.5 {
        view.land = (-view.fall_speed / 9.0).min(1.3);
        sounds.write(crate::fx::PlaySfx::new(Sfx::Land, None));
    }
    view.was_grounded = mover.grounded;
    view.fall_speed = mover.vertical;
    view.land = (view.land - dt * 5.0).max(0.0);
    view.swap = (view.swap - dt * 5.0).max(0.0);
    view.kick = (view.kick - dt * 9.0 * view.kick.max(0.3)).max(0.0);
    view.punch = (view.punch - dt * 5.0).max(0.0);
    view.limb_hit = (view.limb_hit - dt).max(0.0);
    view.shake = (view.shake - dt * 5.0 * view.shake.max(0.25)).clamp(0.0, 2.5);
    view.sway = (view.sway + controls.look * 0.6).clamp_length_max(0.06) * (-dt * 12.0).exp();
    let sprinting = input.held(buttons::SPRINT) && mover.speed > 6.5;
    view.sprint += (sprinting as u8 as f32 - view.sprint) * (1.0 - (-dt * 8.0).exp());
    if let Ok(mut light) = lights.single_mut() {
        light.intensity = if view.flash > 0.0 { 90_000.0 } else { 0.0 };
    }
    view.recoil = (view.recoil - dt * 6.0 * view.recoil.max(0.4)).max(0.0);
    view.flash = (view.flash - dt).max(0.0);
    view.hurt = (view.hurt - dt * 1.6).max(0.0);
    view.blind = (view.blind - dt).max(0.0);
    view.hit = (view.hit - dt).max(0.0);
    let walking = (mover.speed / 5.4).min(1.6) * mover.grounded as u8 as f32;
    view.bob += dt * 9.5 * walking;
    // One footstep per half bob cycle.
    let step = (view.bob / std::f32::consts::PI) as i32;
    if step != view.step {
        view.step = step;
        if walking > 0.3 && health.alive() {
            sounds.write(crate::fx::PlaySfx::new(Sfx::Step, None));
        }
    }

    let stats = content.weapons.get(index).zip(arsenal.weapons.get(index)).map(|(def, state)| resolve(def, state, &content.attachments));
    let interval = content.weapons.get(index).map_or(0.2, |def| def.interval.max(0.05));
    view.cycle = (view.cycle - dt / interval).max(0.0);
    let aiming = input.held(buttons::AIM) && arsenal.reload <= 0.0 && health.alive();
    let quick = match oz_sim::player::perk(&content, me) {
        Some(oz_core::content::PerkEffect::Handling(scale)) => scale,
        _ => 1.0,
    };
    let handling = stats.map_or(1.0, |s| s.handling) * quick;
    view.ads += (aiming as u8 as f32 - view.ads) * (1.0 - (-dt * 15.0 * handling).exp());
    view.scope_kind = stats.map_or(0, |s| s.scope);
    view.scoped = if view.scope_kind > 0 { ((view.ads - 0.8) / 0.2).clamp(0.0, 1.0) } else { 0.0 };

    let ads_fov = stats.map_or(BASE_FOV, |s| s.ads_fov);
    if let Projection::Perspective(perspective) = &mut *projection {
        let base = settings.fov;
        let ads_fov = ads_fov * base / BASE_FOV;
        perspective.fov = (base + (ads_fov - base) * view.ads + 5.0 * view.sprint * (1.0 - view.ads) + view.kick * 0.9 - view.punch * 4.0).to_radians();
    }

    let bob = Vec3::new(0.0, (view.bob * 2.0).sin() * 0.035 * walking * (1.0 - view.ads), 0.0);
    let wanted_eye = if health.alive() { input.eye() } else { 0.45 };
    if view.eye == 0.0 {
        view.eye = wanted_eye;
    }
    view.eye += (wanted_eye - view.eye) * (1.0 - (-dt * 10.0).exp());
    // Peeking out of cover glides the head out past the corner.
    let lean_k = 1.0 - (-dt * 12.0).exp();
    let wanted_lean = if health.alive() { oz_sim::player::lean_offset(&statics, player.translation, input) } else { Vec3::ZERO };
    let lean = view.lean;
    view.lean = lean + (wanted_lean - lean) * lean_k;
    let wanted_third = if cover.third_person() && health.alive() && local { 1.0 } else { 0.0 };
    view.third += (wanted_third - view.third) * (1.0 - (-dt * 9.0).exp());
    let (eye, roll) = (view.eye, if health.alive() { 0.0 } else { 0.5 });
    let t = time.elapsed_secs();
    let shake = Vec3::new((t * 61.0).sin(), (t * 53.0).cos(), (t * 47.0).sin()) * view.shake * 0.012;
    let head = player.translation + Vec3::Y * (eye - view.land * 0.13) + bob + view.lean;
    camera.rotation = Quat::from_euler(EulerRot::YXZ, input.yaw + shake.x, input.pitch + view.recoil * 0.012 + shake.y, roll + shake.z);
    // In cover the camera pulls back over the right shoulder, stopping short of walls.
    let shoulder = camera.rotation * Vec3::new(0.65, 0.35, 2.6) * view.third;
    let length = shoulder.length();
    let pulled = if length > 1e-3 {
        let dir = shoulder / length;
        let reach = statics.raycast(head, dir, length + 0.3).map_or(length, |(d, _)| (d - 0.3).max(0.0)).min(length);
        dir * reach
    } else {
        Vec3::ZERO
    };
    camera.translation = head + pulled;
}

/// The weapon to show in the hands: the one the player asked for if they own it
/// (instant online), otherwise whatever the host says they hold.
pub fn held_weapon(input: &PlayerInput, arsenal: &Arsenal) -> u8 {
    if arsenal.owns(input.weapon as usize) { input.weapon } else { arsenal.current }
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 0 -> 1 -> 0 over `[a, b]`.
fn bump(t: f32, a: f32, b: f32) -> f32 {
    if t <= a || t >= b { 0.0 } else { ((t - a) / (b - a) * PI).sin() }
}

#[allow(clippy::too_many_arguments)]
fn viewmodel(
    time: Res<Time>,
    content: Res<Content>,
    art: Res<Art>,
    mut view: ResMut<ViewState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
    (players, subject): (Query<(&Arsenal, &Health, &Mover, &PlayerInput, Has<LocalPlayer>)>, Res<ViewSubject>),
    mut gun: Query<(Entity, &mut Transform, &mut Visibility), (With<GunRoot>, Without<MuzzleFlash>, Without<GunGroup>)>,
    mut groups: Query<(&GunGroup, &mut Transform, &mut Visibility), (Without<GunRoot>, Without<MuzzleFlash>)>,
    mut flash: Query<(&mut Transform, &mut Visibility), (With<MuzzleFlash>, Without<GunRoot>, Without<GunGroup>)>,
    spectate: Res<crate::spectate::Spectate>,
) {
    let pick = subject.0.and_then(|e| players.get(e).ok()).or_else(|| players.iter().find(|(.., local)| *local));
    let (Some((arsenal, health, mover, input, _)), Ok((root, mut transform, mut visibility))) = (pick, gun.single_mut()) else { return };
    let held = held_weapon(input, arsenal);
    let index = held as usize;
    let (Some(def), Some(state)) = (content.weapons.get(index), arsenal.weapons.get(index)) else { return };
    let stats = resolve(def, state, &content.attachments);

    // Rebuild the hands only when the weapon or its attachments change.
    let signature = (held, state.equipped);
    if view.shown != Some(signature) {
        view.shown = Some(signature);
        let mut model = assemble(def, &state.equipped);
        for (slot, installed) in state.mods.iter().enumerate() {
            if let Some((kind, _)) = installed {
                model = model.cube(kind.paint(), [-0.07, 0.02 + 0.045 * slot as f32, -0.16], [0.025, 0.03, 0.1]);
            }
        }
        let shell = BlockModel::new().cylinder(Paint::Red, [0.0, 0.0, 0.0], [0.035, 0.09, 0.035]).rotated([90.0, 0.0, 0.0]).cylinder(Paint::Gold, [0.0, 0.0, 0.05], [0.037, 0.02, 0.037]).rotated([90.0, 0.0, 0.0]);
        let pieces = [
            (GunGroup::Body, model.filtered(|a| !matches!(a, Anim::Mag | Anim::Slide))),
            (GunGroup::Mag, model.filtered(|a| a == Anim::Mag)),
            (GunGroup::Slide, model.filtered(|a| a == Anim::Slide)),
            (GunGroup::Shell, shell),
        ];
        commands.entity(root).despawn_related::<Children>();
        for (group, piece) in pieces {
            let baked = art.bake(&piece, &mut meshes);
            let holder = commands.spawn((group, Transform::IDENTITY, Visibility::Inherited, ChildOf(root))).id();
            for (mesh, material) in [(&baked.lit, &art.lit), (&baked.unlit, &art.unlit)] {
                if let Some(mesh) = mesh {
                    commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), NotShadowCaster, ChildOf(holder)));
                }
            }
        }
    }

    // A magnified scope fills the view: hide the gun behind the overlay.
    *visibility = if health.alive() && view.scoped < 0.5 && (!spectate.enabled || subject.0.is_some()) && view.third < 0.4 { Visibility::Inherited } else { Visibility::Hidden };
    let t = time.elapsed_secs();
    let walking = (mover.speed / 5.4).min(1.6) * (1.0 - view.ads * 0.94);

    // Reload choreography, chosen by the weapon's reload style.
    let reloading = arsenal.reload > 0.0 && arsenal.reload_total > 0.0;
    let p = if reloading { 1.0 - arsenal.reload / arsenal.reload_total } else { 0.0 };
    let (mut tilt, mut dip, mut roll, mut mag, mut mag_hidden, mut slide, mut shell) = (0.0, 0.0, 0.0, Vec3::ZERO, false, Vec3::ZERO, None);
    if reloading {
        match def.reload_style {
            ReloadStyle::Magazine | ReloadStyle::Belt => {
                let belt = def.reload_style == ReloadStyle::Belt;
                let hold = ease(p / 0.15) * (1.0 - ease((p - 0.85) / 0.15));
                tilt = -0.45 * hold * if belt { -0.6 } else { 1.0 };
                roll = 0.4 * hold;
                dip = 0.08 * hold;
                // Out with the old: drop and fall away; in with the new: rise into the well and slap home.
                if p < 0.35 {
                    mag = Vec3::new(0.0, -0.6 * (p / 0.35).powi(2), 0.05 * p);
                } else if p < 0.45 {
                    mag_hidden = true;
                } else if p < 0.7 {
                    let rise = ease((p - 0.45) / 0.25);
                    mag = Vec3::new(0.08 * (1.0 - rise), -0.45 * (1.0 - rise), 0.0);
                }
                dip -= 0.03 * bump(p, 0.68, 0.76);
                if arsenal.reload_empty {
                    slide = Vec3::Z * 0.07 * bump(p, 0.8, 0.95);
                }
            }
            ReloadStyle::Single => {
                tilt = -0.2;
                roll = 0.55;
                dip = 0.05 + 0.04 * bump(p, 0.45, 0.85);
                // A round travels from below into the loading port.
                let k = ease((p - 0.1) / 0.6);
                shell = Some(Vec3::new(0.06 * (1.0 - k), -0.22 + 0.17 * k, -0.18 + 0.05 * k));
            }
        }
    }
    // Working the action after a shot.
    let c = 1.0 - view.cycle;
    if view.cycle > 0.0 {
        match def.action {
            Action::Bolt => {
                let lift = bump(c, 0.05, 0.9);
                slide = Vec3::new(0.0, 0.03 * lift, 0.09 * bump(c, 0.2, 0.75));
                roll += 0.18 * lift;
                dip += 0.02 * lift;
            }
            Action::Pump => {
                slide = Vec3::Z * 0.12 * bump(c, 0.15, 0.85);
                tilt += 0.06 * bump(c, 0.15, 0.85);
            }
            _ if def.class == GunClass::Pistol => slide = Vec3::Z * 0.06 * bump(c, 0.0, 0.6),
            _ => slide = Vec3::Z * 0.03 * bump(c, 0.0, 0.5),
        }
    }
    // Inspect: turn the gun to show it off, then flip it back.
    let inspect = if view.inspect > 0.0 { 1.0 - view.inspect / 2.2 } else { 0.0 };
    let show = bump(inspect, 0.0, 1.0);
    let flip = bump(inspect, 0.35, 0.75);

    let ads_pos = Vec3::new(0.0, -stats.sight * GUN_SCALE, -0.40);
    let sprint = view.sprint * (1.0 - view.ads);
    transform.translation = GUN_POS.lerp(ads_pos, view.ads)
        + Vec3::new(
            view.bob.sin() * 0.018 * walking + view.sway.x * 0.5 - 0.06 * show + 0.04 * sprint,
            (view.bob * 2.0).sin() * 0.016 * walking - dip - view.swap * 0.22 - view.land * 0.05 - view.sway.y * 0.5 + (t * 1.3).sin() * 0.003 + 0.05 * show - 0.06 * sprint,
            view.recoil * 0.05 + 0.04 * sprint,
        );
    transform.rotation = Quat::from_rotation_y(0.9 * show + 0.55 * sprint)
        * Quat::from_rotation_x(view.recoil * 0.07 + tilt - view.swap * 0.5 - 0.35 * sprint + 0.25 * flip)
        * Quat::from_rotation_z(roll + view.sway.x * 1.6 + 1.2 * flip + 0.25 * sprint);

    for (group, mut piece, mut seen) in &mut groups {
        let (offset, visible) = match group {
            GunGroup::Body => (Vec3::ZERO, true),
            GunGroup::Mag => (mag, !mag_hidden),
            GunGroup::Slide => (slide, true),
            GunGroup::Shell => (shell.unwrap_or(Vec3::ZERO), shell.is_some()),
        };
        piece.translation = offset;
        let wanted = if visible { Visibility::Inherited } else { Visibility::Hidden };
        if *seen != wanted {
            *seen = wanted;
        }
    }

    if let Ok((mut flash_transform, mut flash_visibility)) = flash.single_mut() {
        let lit = view.flash > 0.0 && health.alive() && def.mag > 0 && view.scoped < 0.5 && !stats.quiet && !spectate.enabled;
        *flash_visibility = if lit { Visibility::Inherited } else { Visibility::Hidden };
        flash_transform.translation = transform.translation + transform.rotation * (Vec3::new(0.0, 0.02, -stats.length - 0.08) * GUN_SCALE);
        flash_transform.scale = Vec3::new(0.06, 0.06, 0.15) * (0.7 + view.flash * 14.0);
        flash_transform.rotation = Quat::from_rotation_z(t * 31.0);
    }
}
