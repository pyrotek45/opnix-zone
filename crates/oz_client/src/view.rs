//! The first-person view: cameras, the low-resolution render target and the viewmodel.
//!
//! The 3D world is rendered into a small texture (window size divided by the
//! `pixel_size` setting) and blown up with nearest-neighbour filtering. That
//! is where both the chunky look and most of the frame rate come from: at
//! pixel size 3 the GPU shades one ninth of the pixels.

use crate::{ClientSet, art::Art, retro::Retro, scene::Sky, settings::Settings};
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
use oz_core::arsenal::{Action, GunClass, ReloadStyle, SLOT_COUNT, SightLine, assemble, resolve, sight_line};
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

/// The off hand that cooks and throws grenades in first person.
#[derive(Component)]
struct ThrowArm;

/// The grenade in that hand (hidden once it has left it).
#[derive(Component)]
struct ThrowGrenade;

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
    /// Last frame's stance, aim and reload progress, for one-shot sounds on changes.
    last_stance: u8,
    was_aiming: bool,
    reload_p: f32,
    step: i32,
    last_weapon: Option<u8>,
    last_shots: u32,
    /// (weapon, equipped parts) currently shown in the hands.
    shown: Option<(u8, [u8; SLOT_COUNT])>,
    /// How far ahead of the eye the shown gun's rear sight sits when aimed ([`eye_relief`]).
    relief: f32,
    /// Smoothed eye height (crouching and going prone glide).
    eye: f32,
    /// How far the eye still trails the feet after a stair step snapped them up or down
    /// (it glides after them instead), and where the feet were last frame.
    stair: f32,
    feet: Option<Vec3>,
    /// Smoothed peek offset out of cover.
    lean: Vec3,
    /// 0 = first person, 1 = over the shoulder (in cover).
    pub third: f32,
    /// Which shoulder the cover camera is over: -1 left, 1 right (eased), and how far
    /// out past a wall's end it leans to see round the corner (0..1).
    pub shoulder: f32,
    pub round: f32,
    /// A quick zoom-in on a kill, decaying from 1.
    pub punch: f32,
    /// The last hit marker was a headshot.
    pub head_hit: bool,
    /// Seconds left of the "limb off" marker flare.
    pub limb_hit: f32,
    /// Where the gun's muzzle is, in camera space (tracers start here).
    pub muzzle: Vec3,
    /// Per-shot random twist of the gun and flash, so automatic fire feels alive.
    kick_roll: f32,
    kick_side: f32,
    shot_count: u32,
    /// 1 right after a shot, decaying over the weapon's cycle time (bolt / pump / slide animation).
    cycle: f32,
    /// Seconds left of the weapon inspect animation.
    pub inspect: f32,
    /// 0..1: how far the gun is pulled back from a wall right in front of you.
    pub tuck: f32,
    /// 0..1: on a ladder (gun slung, hand-over-hand rhythm).
    pub climb: f32,
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
            .add_systems(Update, (follow_player, viewmodel, camera_tour).chain().in_set(FollowPlayer).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

/// A developer's camera tour for looking the maps over: with `OZ_TOUR` set, the camera
/// leaves the player and visits viewpoints in turn, `OZ_TOUR_SECS` (default 3) seconds each,
/// printing each one as it gets there. `OZ_TOUR=auto` circles the arena on a grid, looking
/// down at the streets; otherwise it is a list of `x,y,z,tx,ty,tz` (eye, then target) split by `;`.
fn camera_tour(
    time: Res<Time>,
    map: Res<Map>,
    mut cameras: Query<&mut Transform, With<WorldCamera>>,
    mut guns: Query<&mut Visibility, With<GunRoot>>,
    mut stops: Local<Option<Vec<(Vec3, Vec3)>>>,
    mut at: Local<(f32, usize)>,
) {
    let stops = stops.get_or_insert_with(|| {
        let Ok(tour) = std::env::var("OZ_TOUR") else { return Vec::new() };
        if tour == "auto" {
            let r = map.0.arena * 0.75;
            let mut out = Vec::new();
            for j in 0..5 {
                for i in 0..5 {
                    let p = Vec2::new(-r + 2.0 * r * i as f32 / 4.0, -r + 2.0 * r * j as f32 / 4.0);
                    let look = p + Vec2::from_angle((i * 5 + j) as f32 * 2.4) * 22.0;
                    let ground = map.0.height(p.x, p.y);
                    out.push((Vec3::new(p.x, ground + 13.0, p.y), Vec3::new(look.x, map.0.height(look.x, look.y) + 1.0, look.y)));
                }
            }
            return out;
        }
        tour.split(';')
            .filter_map(|stop| {
                let v: Vec<f32> = stop.split(',').filter_map(|n| n.trim().parse().ok()).collect();
                (v.len() == 6).then(|| (Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5])))
            })
            .collect()
    });
    if stops.is_empty() {
        return;
    }
    let secs = std::env::var("OZ_TOUR_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(3.0f32);
    let (clock, shown) = &mut *at;
    *clock += time.delta_secs();
    let index = (*clock / secs) as usize % stops.len();
    if index != *shown || *clock < time.delta_secs() * 1.5 {
        *shown = index;
        eprintln!("OZ_TOUR stop {index} {:?}", stops[index]);
    }
    let (eye, target) = stops[index];
    if let Ok(mut camera) = cameras.single_mut() {
        *camera = Transform::from_translation(eye).looking_at(target, Vec3::Y);
    }
    for mut gun in &mut guns {
        *gun = Visibility::Hidden;
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
        && let Some(image) = images.get_mut(&target.0)
    {
        image.resize(Extent3d { width, height, depth_or_array_layers: 1 });
    }
}

/// Spawned by `scene` once the sky exists.
pub fn spawn_camera(mut commands: Commands, target: Res<WorldTarget>, sky: Res<Sky>, art: Res<Art>, mut view: ResMut<ViewState>, mut meshes: ResMut<Assets<Mesh>>) {
    *view = ViewState::default();
    // A robot forearm and hand, and a grenade to put in it.
    let arm = art.bake(
        &BlockModel::new().segment(Paint::Steel, [0.0, -0.05, 0.32], [0.0, 0.0, 0.0], 0.075).cube(Paint::Dark, [0.0, 0.01, -0.02], [0.1, 0.09, 0.11]).shard(
            Paint::Aether,
            [0.0, -0.03, 0.2],
            [0.05, 0.05, 0.05],
        ),
        &mut meshes,
    );
    let grenade = art.bake(
        &BlockModel::new().cylinder(Paint::Iron, [0.0, 0.06, -0.03], [0.07, 0.1, 0.07]).cube(Paint::Steel, [0.0, 0.12, -0.03], [0.03, 0.03, 0.03]).cube(
            Paint::Gold,
            [0.03, 0.12, -0.03],
            [0.02, 0.05, 0.01],
        ),
        &mut meshes,
    );
    commands
        .spawn((
            MatchEntity,
            WorldCamera,
            Camera3d::default(),
            Camera { target: RenderTarget::Image(target.0.clone().into()), order: 0, ..default() },
            Projection::Perspective(PerspectiveProjection { fov: BASE_FOV.to_radians(), near: 0.1, far: 600.0, ..default() }),
            Msaa::Off,
            Tonemapping::Reinhard,
            Retro::default(),
            DistanceFog { color: Color::srgb(0.65, 0.68, 0.67), falloff: FogFalloff::Linear { start: 60.0, end: 230.0 }, ..default() },
            Skybox { image: sky.day.clone(), brightness: 1000.0, ..default() },
            Transform::from_xyz(0.0, 30.0, 0.0),
        ))
        .with_children(|camera| {
            camera.spawn((GunRoot, Transform::from_translation(GUN_POS).with_scale(Vec3::splat(GUN_SCALE)), Visibility::Inherited));
            camera.spawn((ThrowArm, Transform::from_xyz(-0.3, -0.6, -0.4), Visibility::Hidden)).with_children(|hand| {
                for (mesh, material) in [(&arm.lit, &art.lit), (&arm.unlit, &art.unlit)] {
                    if let Some(mesh) = mesh {
                        hand.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), NotShadowCaster));
                    }
                }
                hand.spawn((ThrowGrenade, Transform::IDENTITY, Visibility::Inherited)).with_children(|g| {
                    for (mesh, material) in [(&grenade.lit, &art.lit), (&grenade.unlit, &art.unlit)] {
                        if let Some(mesh) = mesh {
                            g.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), NotShadowCaster));
                        }
                    }
                });
            });
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
    (role, prediction, settings, statics, cover, killcam): (Res<NetRole>, Res<crate::predict::Prediction>, Res<crate::settings::Settings>, Res<Statics>, Res<crate::cover::Cover>, Res<crate::killcam::KillCam>),
) {
    let dt = time.delta_secs().min(0.05);
    // A kill cam replay: the killer's own first-person view, from the recording.
    let base = players.iter().find(|(.., local)| *local);
    let replay = killcam.pov.as_ref().zip(base).map(|(pov, (_, _, arsenal, _, _, me, _))| pov.stand_in(arsenal, me));
    let pick = match &replay {
        Some((t, i, a, m, h, p)) => Some((t, i, a, m, h, p, false)),
        None => subject.0.and_then(|e| players.get(e).ok()).or_else(|| players.iter().find(|(.., local)| *local)),
    };
    let (Some((player, input, arsenal, mover, health, me, local)), Ok((mut camera, mut projection))) = (pick, cameras.single_mut()) else {
        return;
    };

    // Recoil kicks whenever the shot counter advances: predicted locally when online.
    let index = held_weapon(input, arsenal) as usize;
    let shots = if *role == NetRole::Client && local { prediction.total } else { arsenal.shots };
    if shots != view.last_shots {
        view.last_shots = shots;
        if let Some((def, state)) = content.weapons.get(index).zip(arsenal.weapons.get(index)) {
            // Exactly the kick the aim gets (see `input`): the gun's recoil after its parts,
            // scaled by its class, braced by stance and by aiming. Pistols and SMGs flick,
            // rifles shove, LMGs, shotguns and sniper rifles slam.
            let stats = resolve(def, state, &content.attachments);
            let recoil = oz_core::arsenal::shot_kick(def.class, stats.recoil, stance::recoil(input.stance), input.held(buttons::AIM));
            view.recoil = (view.recoil + recoil).min(oz_core::arsenal::MAX_KICK);
            view.flash = 0.05;
            view.shake += recoil * 0.1;
            view.kick = (view.kick + recoil * 0.9).min(3.5);
            view.cycle = 1.0;
            // Each shot twists and shoves the gun a little differently.
            view.shot_count = view.shot_count.wrapping_add(1);
            let count = view.shot_count;
            let r = |k: u32| ((count.wrapping_mul(2654435761).wrapping_add(k)) >> 16) as f32 / 65536.0 * 2.0 - 1.0;
            view.kick_roll = (view.kick_roll + r(1) * 0.09 * recoil.sqrt()).clamp(-0.25, 0.25);
            view.kick_side = (view.kick_side + r(2) * 0.012 * recoil.sqrt()).clamp(-0.04, 0.04);
            view.shake += recoil * 0.12;
            // Brass hits the floor a moment later.
            if def.mode == oz_core::arsenal::FireMode::Hitscan && def.mag > 0 {
                sounds.write(crate::fx::PlaySfx::new(Sfx::Shell, None).quiet(0.35).later(0.38));
            }
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
    // Leaving the ground under your own power.
    if view.was_grounded && !mover.grounded && mover.vertical > 2.0 {
        sounds.write(crate::fx::PlaySfx::new(Sfx::Jump, None));
    }
    // Kit rustles when you change stance or shoulder the gun.
    if input.stance != view.last_stance {
        view.last_stance = input.stance;
        sounds.write(crate::fx::PlaySfx::new(Sfx::Cloth, None).quiet(if input.stance == stance::PRONE { 1.0 } else { 0.7 }));
    }
    let aiming_now = input.held(buttons::AIM);
    if aiming_now && !view.was_aiming {
        sounds.write(crate::fx::PlaySfx::new(Sfx::Cloth, None).quiet(0.4));
    }
    view.was_aiming = aiming_now;
    view.was_grounded = mover.grounded;
    view.fall_speed = mover.vertical;
    view.land = (view.land - dt * 5.0).max(0.0);
    view.swap = (view.swap - dt * 5.0).max(0.0);
    settle_kick(&mut view, dt);
    view.punch = (view.punch - dt * 5.0).max(0.0);
    view.limb_hit = (view.limb_hit - dt).max(0.0);
    view.shake = (view.shake - dt * 5.0 * view.shake.max(0.25)).clamp(0.0, 2.5);
    view.sway = (view.sway + controls.look * 0.6).clamp_length_max(0.06) * (-dt * 12.0).exp();
    // The slowest, heaviest kit still sprints past 6 m/s.
    let sprinting = input.held(buttons::SPRINT) && mover.speed > 6.0;
    view.sprint += (sprinting as u8 as f32 - view.sprint) * (1.0 - (-dt * 8.0).exp());
    if let Ok(mut light) = lights.single_mut() {
        light.intensity = if view.flash > 0.0 { 90_000.0 } else { 0.0 };
    }
    view.flash = (view.flash - dt).max(0.0);
    view.hurt = (view.hurt - dt * 1.6).max(0.0);
    view.blind = (view.blind - dt).max(0.0);
    view.hit = (view.hit - dt).max(0.0);
    let walking = (mover.speed / 5.4).min(1.6) * mover.grounded as u8 as f32 * (!mover.climbing) as u8 as f32;
    view.bob += dt * 9.5 * walking + if mover.climbing { dt * 7.0 } else { 0.0 };
    view.climb += ((mover.climbing && local) as u8 as f32 - view.climb) * (1.0 - (-dt * 10.0).exp());
    // One footstep per half bob cycle; what it sounds like depends on how you move and what is underfoot.
    let step = (view.bob / std::f32::consts::PI) as i32;
    if step != view.step {
        view.step = step;
        if walking > 0.15 && health.alive() {
            let feet = player.translation;
            let on_floor = feet.y - statics.support(feet.x, feet.z, feet.y + 0.1) < 0.06;
            let (sfx, volume) = match input.stance {
                stance::PRONE => (Sfx::Crawl, 0.6),
                stance::CROUCH => (Sfx::StepSoft, 0.7),
                _ if sprinting => (Sfx::StepHeavy, 1.0),
                _ if on_floor => (Sfx::StepWood, 0.9),
                _ => (Sfx::Step, 0.9),
            };
            sounds.write(crate::fx::PlaySfx::new(sfx, None).quiet(volume));
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
    // Walking up or down stairs the feet jump a step at a time: steeper than any slope
    // underfoot, so a rise much bigger than the ground covered is a step, and the eye glides it.
    let feet = player.translation;
    let (rise, moved) = view.feet.map_or((0.0, f32::MAX), |was| (feet.y - was.y, was.xz().distance(feet.xz())));
    view.feet = Some(feet);
    if !local || moved > 1.5 || mover.climbing {
        view.stair = 0.0;
    } else if mover.grounded && rise.abs() > moved * 0.9 + 0.02 && rise.abs() < 0.6 {
        view.stair = (view.stair - rise).clamp(-0.5, 0.5);
    }
    view.stair *= (-dt * 14.0).exp();
    // Peeking out of cover glides the head out past the corner.
    let lean_k = 1.0 - (-dt * 12.0).exp();
    let wanted_lean = if health.alive() { oz_sim::player::lean_offset(&statics, player.translation, input) } else { Vec3::ZERO };
    let lean = view.lean;
    view.lean = lean + (wanted_lean - lean) * lean_k;
    let wanted_third = if cover.third_person() && health.alive() && local { 1.0 } else { 0.0 };
    view.third += (wanted_third - view.third) * (1.0 - (-dt * 9.0).exp());
    // Hauling up a ladder rocks the view a little with each pull.
    let (eye, roll) = (view.eye, if health.alive() { (view.bob).sin() * 0.04 * view.climb } else { 0.5 });
    let t = time.elapsed_secs();
    let shake = Vec3::new((t * 61.0).sin(), (t * 53.0).cos(), (t * 47.0).sin()) * view.shake * 0.012;
    let head = player.translation + Vec3::Y * (eye - view.land * 0.13 + view.stair) + bob + view.lean;
    // Holding a gun up is never perfectly still: a slow drift down the sights, less when crouched or prone.
    let hold = view.ads * stance::steadiness(input.stance) * 0.0035;
    let drift = Vec2::new((t * 0.8).sin() + 0.5 * (t * 1.9).sin(), (t * 1.3).cos() * 0.7) * hold;
    camera.rotation = Quat::from_euler(EulerRot::YXZ, input.yaw + shake.x + drift.x, input.pitch + view.recoil * oz_core::arsenal::RECOIL_RADIANS * 1.6 + shake.y + drift.y, roll + shake.z);
    // In cover the camera pulls back over a shoulder, stopping short of walls. Which
    // shoulder follows where you look along the wall, and at the wall's end it swings out
    // past the corner so you see round it instead of into the wall.
    if view.shoulder == 0.0 {
        view.shoulder = 1.0;
    }
    let (wanted_side, wanted_round) = match cover.wall {
        Some(wall) => {
            let along = Vec3::new(-wall.normal.z, 0.0, wall.normal.x);
            let look = Quat::from_rotation_y(input.yaw) * Vec3::NEG_Z;
            let right = Quat::from_rotation_y(input.yaw) * Vec3::X;
            let toward = look.dot(along);
            // Looking along the wall picks the shoulder on that side of the screen; looking
            // square on keeps the shoulder we had.
            let looked = if toward.abs() > 0.2 { (along * toward.signum()).dot(right).signum() } else { view.shoulder.signum() };
            // At a window or a corner the building decides: look out over the shoulder on the
            // side that is open at head height, not the one with wall in front of it.
            let head = player.translation + Vec3::Y * 1.55;
            let open_at = |side: f32| statics.raycast(head + along * side * 0.55, -wall.normal, 1.2).is_none();
            let (left_open, right_open) = (open_at(-1.0), open_at(1.0));
            let screen = match (left_open, right_open) {
                (true, false) => (along * -1.0).dot(right).signum(),
                (false, true) => along.dot(right).signum(),
                _ => looked,
            };
            // Which end of the wall that is (along -1 or +1), and whether we are at it.
            let end = if along.dot(right) * screen >= 0.0 { 1 } else { 0 };
            let at_edge = cover.edges[end];
            (if screen == 0.0 { view.shoulder.signum() } else { screen }, if at_edge { 1.0 } else { 0.0 })
        }
        None => (view.shoulder.signum(), 0.0),
    };
    let ease = 1.0 - (-dt * 6.0).exp();
    view.shoulder += (wanted_side - view.shoulder) * ease;
    view.round += (wanted_round - view.round) * ease;
    let shoulder = camera.rotation * Vec3::new((1.15 + 0.7 * view.round) * view.shoulder, 0.45, 2.3) * view.third;
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

/// The visual recoil dying away after shots.
fn settle_kick(view: &mut ViewState, dt: f32) {
    view.kick = (view.kick - dt * 9.0 * view.kick.max(0.3)).max(0.0);
    view.kick_roll *= (-dt * 10.0).exp();
    view.kick_side *= (-dt * 12.0).exp();
    view.recoil = (view.recoil - dt * 6.0 * view.recoil.max(0.4)).max(0.0);
}

/// How far ahead of the eye a wall makes a gun of this length tuck its muzzle up.
fn tuck_reach(length: f32) -> f32 {
    0.95 + length * 0.3
}

/// How far the gun tucks with a wall `room` metres ahead (`None`: nothing within reach).
/// Open space never tucks, however long the gun: a long barrel used to read "no hit" as
/// a wall at the end of a fixed-length probe and aim its sights off the screen centre.
fn wanted_tuck(length: f32, room: Option<f32>) -> f32 {
    let reach = tuck_reach(length);
    ((reach - room.unwrap_or(reach).min(reach)) / 0.6).clamp(0.0, 1.0)
}

/// The rotation, in camera space, that turns the camera's forward axis onto the direction
/// the shots actually go.
fn shot_in_view(camera: Quat, aim_dir: Vec3) -> Quat {
    let dir = (camera.inverse() * aim_dir).normalize_or(Vec3::NEG_Z);
    Quat::from_rotation_arc(Vec3::NEG_Z, dir)
}

/// Nearest the eye any part of a gun may come down the sights while still in view: just
/// past the camera's 0.1 m near plane, so nothing is sliced open across the screen.
const NEAR_CLEAR: f32 = 0.125;
/// How close the sights come to the eye when the gun allows it: the rear notch (or the back
/// of an optic) this far ahead, big and close like a real sight picture.
const EYE_RELIEF: [f32; 2] = [0.24, 0.28];

/// How far ahead of the eye a gun's rear sight (or optic) sits when aimed: as close as
/// [`EYE_RELIEF`] wants unless some part of it (a stock's comb, a carry handle, a charging
/// handle) would then come nearer the eye than [`NEAR_CLEAR`] inside the view; then just
/// far enough out that it does not. A stock passes below and behind the eye; on screen it
/// must not be cut off by the near plane.
pub fn eye_relief(model: &BlockModel, sight: &SightLine) -> f32 {
    let rotation = Quat::from_rotation_arc(sight.dir(), Vec3::NEG_Z);
    let points: Vec<Vec3> = model
        .parts
        .iter()
        .flat_map(oz_core::model::part_points)
        .chain(model.hulls.iter().flat_map(|h| h.points.iter().copied()))
        .map(|p| rotation * (p - sight.rear) * GUN_SCALE)
        .collect();
    // The widest view the gun can be seen in: the hip field of view on an ultrawide screen.
    let (up, side) = ((BASE_FOV * 0.5).to_radians().tan() * 1.25, (BASE_FOV * 0.5).to_radians().tan() * 2.6);
    let clear = |relief: f32| {
        points.iter().all(|p| {
            let depth = relief - p.z;
            let at = depth.max(0.1);
            depth >= NEAR_CLEAR || p.y.abs() > at * up || p.x.abs() > at * side
        })
    };
    let mut relief = EYE_RELIEF[sight.optic as usize];
    while !clear(relief) && relief < 0.8 {
        relief += 0.005;
    }
    relief
}

/// Where the gun sits fully aimed, in camera space: its sight line (the rear notch and the
/// front post, or the optic's axis) along the camera's forward axis, so the post's tip, the
/// dot or the lens centre is on the screen centre and the barrel runs straight away from
/// the eye, with the rear sight `relief` metres ahead of it (see [`eye_relief`]).
pub fn ads_pose(sight: &SightLine, relief: f32) -> Transform {
    let rotation = Quat::from_rotation_arc(sight.dir(), Vec3::NEG_Z);
    let rear = rotation * sight.rear * GUN_SCALE;
    Transform { translation: Vec3::new(0.0, 0.0, -relief) - rear, rotation, scale: Vec3::splat(GUN_SCALE) }
}

/// What moves the gun in the hands this frame apart from the [`ViewState`]: the clock,
/// walking, inspecting, sprinting, reload and action choreography, the off hand and melee.
#[derive(Clone, Copy, Default)]
struct Hold {
    t: f32,
    walking: f32,
    show: f32,
    flip: f32,
    sprint: f32,
    dip: f32,
    tilt: f32,
    roll: f32,
    busy: f32,
    melee: f32,
}

/// The gun's pose in camera space: from the hip to [`ads_pose`] by `view.ads`, with
/// everything that moves it on top. `aim` turns the camera's axis onto the shots (see
/// [`shot_in_view`]); down the sights the gun follows it, so the sight picture is where
/// the bullets land.
fn gun_pose(view: &ViewState, sight: &SightLine, hold: &Hold, aim: Quat) -> Transform {
    let Hold { t, walking, show, flip, sprint, dip, tilt, roll, busy, melee } = *hold;
    let ads = ads_pose(sight, if view.relief > 0.0 { view.relief } else { EYE_RELIEF[sight.optic as usize] });
    // Idle life (breathing, a slow wobble) fades out down the sights: moving the gun
    // without turning it would pull the post off the notch. The hold drift on the camera
    // keeps the sight picture alive instead. Mouse lag is mostly gone there too.
    let idle = 1.0 - view.ads;
    let sway = view.sway * (1.0 - 0.8 * view.ads);
    // Breathing: the gun drifts in a slow figure eight.
    let breath = Vec3::new((t * 0.9).sin() * 0.006, (t * 1.8).sin() * 0.004, 0.0) * idle;
    let translation = GUN_POS.lerp(ads.translation, view.ads)
        + breath
        + Vec3::new(
            view.bob.sin() * 0.018 * walking + sway.x * 0.5 - 0.06 * show + 0.04 * sprint,
            (view.bob * 2.0).sin() * 0.016 * walking - dip - view.swap * 0.22 - view.land * 0.05 - sway.y * 0.5 + (t * 1.3).sin() * 0.003 * idle + 0.05 * show - 0.06 * sprint,
            view.recoil.min(3.0) * 0.07 + 0.04 * sprint,
        )
        + Vec3::X * view.kick_side
        + Vec3::new(0.12, -0.32, 0.05) * busy
        + Vec3::new(0.04, -0.1, 0.3) * view.tuck
        + Vec3::new(0.14, -0.5, 0.12) * view.climb
        + Vec3::new(-0.08, 0.04, -0.2) * melee;
    let rotation = Quat::from_rotation_y(0.9 * show + 0.55 * sprint + view.kick_side * 2.0 + 0.6 * melee)
        * Quat::from_rotation_x(view.recoil * 0.075 + tilt - view.swap * 0.5 - 0.35 * sprint + 0.25 * flip - 0.5 * busy + 0.35 * melee + 0.9 * view.tuck - 0.7 * view.climb)
        * Quat::from_rotation_z(roll + sway.x * 1.6 + 1.2 * flip + 0.25 * sprint + view.kick_roll + (t * 0.7).sin() * 0.012 * idle)
        * Quat::IDENTITY.slerp(ads.rotation, view.ads);
    let follow = Quat::IDENTITY.slerp(aim, view.ads);
    Transform { translation: follow * translation, rotation: follow * rotation, scale: Vec3::splat(GUN_SCALE) }
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
    mut sounds: EventWriter<crate::fx::PlaySfx>,
    (hands, mut arm, mut nade): (
        Res<crate::tactical::Hands>,
        Query<(&mut Transform, &mut Visibility), (With<ThrowArm>, Without<GunRoot>, Without<GunGroup>, Without<MuzzleFlash>, Without<ThrowGrenade>)>,
        Query<&mut Visibility, (With<ThrowGrenade>, Without<ThrowArm>, Without<GunRoot>, Without<GunGroup>, Without<MuzzleFlash>)>,
    ),
    (statics, eyes, killcam, me): (Res<Statics>, Query<&Transform, (With<WorldCamera>, Without<GunRoot>, Without<GunGroup>, Without<MuzzleFlash>, Without<ThrowArm>)>, Res<crate::killcam::KillCam>, Query<&Player, With<LocalPlayer>>),
) {
    // A kill cam replay shows the killer's gun, in their hands, as they held it.
    let base = players.iter().find(|(.., local)| *local).zip(me.single().ok());
    let replay = killcam.pov.as_ref().zip(base).map(|(pov, ((arsenal, ..), me))| pov.stand_in(arsenal, me));
    let pick = match &replay {
        Some((_, i, a, m, h, _)) => Some((a, h, m, i, false)),
        None => subject.0.and_then(|e| players.get(e).ok()).or_else(|| players.iter().find(|(.., local)| *local)),
    };
    let (Some((arsenal, health, mover, input, _)), Ok((root, mut transform, mut visibility))) = (pick, gun.single_mut()) else {
        return;
    };
    let held = held_weapon(input, arsenal);
    let index = held as usize;
    let (Some(def), Some(state)) = (content.weapons.get(index), arsenal.weapons.get(index)) else {
        return;
    };
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
        view.relief = eye_relief(&model, &sight_line(def, &state.equipped));
        let shell = BlockModel::new()
            .cylinder(Paint::Red, [0.0, 0.0, 0.0], [0.035, 0.09, 0.035])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Gold, [0.0, 0.0, 0.05], [0.037, 0.02, 0.037])
            .rotated([90.0, 0.0, 0.0]);
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
    // The reload's mechanical beats, timed to the animation.
    let crossed = |at: f32| view.reload_p < at && p >= at;
    if reloading {
        match def.reload_style {
            ReloadStyle::Single => {
                if crossed(0.55) {
                    sounds.write(crate::fx::PlaySfx::new(Sfx::MagIn, None).quiet(0.6));
                }
            }
            _ => {
                if crossed(0.12) {
                    sounds.write(crate::fx::PlaySfx::new(Sfx::MagOut, None));
                }
                if crossed(0.7) {
                    sounds.write(crate::fx::PlaySfx::new(Sfx::MagIn, None));
                }
                if arsenal.reload_empty && crossed(0.86) {
                    sounds.write(crate::fx::PlaySfx::new(Sfx::Bolt, None));
                }
            }
        }
    }
    view.reload_p = p;
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
                // Which way the magazine leaves: down out of the well, up off a top-fed gun,
                // or a revolver cylinder swinging out to the side.
                let away = match def.frame.map(|f| f.mag) {
                    Some(oz_core::arsenal::MagShape::Top) => Vec3::new(0.0, 0.5, 0.12),
                    Some(oz_core::arsenal::MagShape::Cylinder) => Vec3::new(-0.3, -0.12, 0.0),
                    _ => Vec3::new(0.0, -0.6, 0.0),
                };
                if p < 0.35 {
                    mag = away * (p / 0.35).powi(2) + Vec3::Z * 0.05 * p;
                } else if p < 0.45 {
                    mag_hidden = true;
                } else if p < 0.7 {
                    let rise = ease((p - 0.45) / 0.25);
                    mag = away * 0.75 * (1.0 - rise) + Vec3::X * 0.08 * (1.0 - rise);
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

    // The off hand: up with a cooking grenade, a big overarm throw, or back down with the pin in.
    // The gun drops out of the way while the hand is busy; a melee shoves it forward, stock first.
    let own = subject.0.is_none() && health.alive() && view.third < 0.4;
    let throw_t = hands.since_throw / 0.45;
    let cancel_t = hands.since_cancel / 0.3;
    let (arm_pose, arm_shown, grenade_shown) = if !own {
        (None, false, false)
    } else if let Some(cooked) = hands.cooking {
        // Raised beside the face, wound back a little more the longer it cooks.
        let wind = ease(cooked / 0.35);
        let shake = if hands.live && hands.fuse - cooked < 0.6 { (t * 40.0).sin() * 0.006 } else { 0.0 };
        (Some((Vec3::new(-0.22 + shake, -0.42 + 0.3 * wind, -0.42 + 0.06 * wind), -0.5 * wind)), true, true)
    } else if throw_t < 1.0 {
        // Over the top and away; the grenade leaves the hand early in the swing.
        let k = ease(throw_t);
        let swing = Vec3::new(-0.22 + 0.12 * k, -0.12 + 0.12 * bump(throw_t, 0.0, 0.6) - 0.45 * ease((throw_t - 0.5) / 0.5), -0.36 - 0.32 * bump(throw_t, 0.0, 0.8));
        (Some((swing, -0.5 + 1.9 * k)), true, throw_t < 0.25)
    } else if cancel_t < 1.0 {
        (Some((Vec3::new(-0.22, -0.12 - 0.5 * ease(cancel_t), -0.42), -0.5 + 0.5 * cancel_t)), true, true)
    } else {
        (None, false, false)
    };
    let busy = if hands.cooking.is_some() {
        1.0
    } else if throw_t < 1.0 {
        1.0 - ease((throw_t - 0.6) / 0.4)
    } else if cancel_t < 1.0 {
        1.0 - cancel_t
    } else {
        0.0
    };
    let melee_t = hands.since_melee / 0.42;
    let melee = if own && melee_t < 1.0 { bump(melee_t, 0.0, 1.0) } else { 0.0 };
    if let Ok((mut arm_transform, mut arm_visibility)) = arm.single_mut() {
        let wanted = if arm_shown { Visibility::Inherited } else { Visibility::Hidden };
        if *arm_visibility != wanted {
            *arm_visibility = wanted;
        }
        if let Some((at, pitch)) = arm_pose {
            arm_transform.translation = at;
            arm_transform.rotation = Quat::from_rotation_x(pitch) * Quat::from_rotation_z(-0.25);
        }
    }
    if let Ok(mut visibility) = nade.single_mut() {
        let wanted = if grenade_shown { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }

    // Up against a wall the gun pulls back and points its muzzle up instead of sinking into it.
    let reach = tuck_reach(stats.length);
    let room = eyes.single().ok().and_then(|eye| statics.raycast(eye.translation, eye.forward().as_vec3(), reach).map(|(d, _)| d));
    let tuck_rate = 1.0 - (-time.delta_secs() * 12.0).exp();
    view.tuck += (wanted_tuck(stats.length, room) - view.tuck) * tuck_rate;
    // Down the sights the gun points where the shots go (the aim, not the camera, which
    // the visual kick, shake and hold drift move about); with those settled that is dead centre.
    let aim = eyes.single().ok().map_or(Quat::IDENTITY, |eye| shot_in_view(eye.rotation, input.aim_dir()));
    let hold = Hold { t, walking, show, flip, sprint: view.sprint * (1.0 - view.ads), dip, tilt, roll, busy, melee };
    let pose = gun_pose(&view, &sight_line(def, &state.equipped), &hold, aim);
    (transform.translation, transform.rotation) = (pose.translation, pose.rotation);
    // Tracers leave from here, sights up or not.
    view.muzzle = transform.translation + transform.rotation * (Vec3::new(0.0, 0.02, -stats.length - 0.05) * GUN_SCALE);

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
        // A big, ragged star of flame that is different every shot.
        let size = 0.8 + view.flash * 18.0 * (1.0 + 0.3 * (view.shot_count as f32 * 1.7).sin());
        flash_transform.scale = Vec3::new(0.07, 0.07, 0.2) * size;
        flash_transform.rotation = transform.rotation * Quat::from_rotation_z(view.shot_count as f32 * 2.399);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::camera::CameraProjection;
    use oz_core::arsenal::{WeaponDef, WeaponState, options, slot, standard_attachments};

    const SCREEN: Vec2 = Vec2::new(1280.0, 720.0);

    /// Pixels from the screen centre at which a camera-space point is drawn.
    fn on_screen(point: Vec3, fov: f32) -> Vec2 {
        let projection = PerspectiveProjection { fov: fov.to_radians(), aspect_ratio: SCREEN.x / SCREEN.y, near: 0.1, far: 600.0 };
        let clip = projection.get_clip_from_view() * point.extend(1.0);
        assert!(clip.w > 0.1, "{point} is behind the near plane");
        clip.truncate().truncate() / clip.w * SCREEN * 0.5
    }

    /// Every gun with every sight, and each barrel and muzzle device that moves the front post,
    /// plus every stock, grip and magazine (each alongside every sight).
    fn builds() -> Vec<(WeaponDef, [u8; SLOT_COUNT])> {
        let table = standard_attachments();
        let mut out = Vec::new();
        for gun in oz_core::kit::standard_guns() {
            let fits = |s: usize| options(&gun, &table, s).into_iter().map(|o| o.0 as u8).collect::<Vec<_>>();
            for optic in fits(slot::OPTIC) {
                for barrel in fits(slot::BARREL) {
                    for muzzle in fits(slot::MUZZLE) {
                        let mut parts = gun.defaults;
                        (parts[slot::OPTIC], parts[slot::BARREL], parts[slot::MUZZLE]) = (optic, barrel, muzzle);
                        out.push((gun.clone(), parts));
                    }
                }
                for other in [slot::UNDER, slot::MAGAZINE, slot::STOCK] {
                    for variant in fits(other) {
                        let mut parts = gun.defaults;
                        (parts[slot::OPTIC], parts[other]) = (optic, variant);
                        out.push((gun.clone(), parts));
                    }
                }
            }
        }
        out
    }

    fn ads_fov(def: &WeaponDef, parts: [u8; SLOT_COUNT]) -> f32 {
        resolve(def, &WeaponState { equipped: parts, ..WeaponState::fresh(def) }, &standard_attachments()).ads_fov
    }

    /// Where the sights point on screen (the front post's tip, the dot or the lens centre, and
    /// the rear notch), and how far (radians) the sight line is from parallel to the view.
    fn sight_picture(pose: &Transform, line: &SightLine, fov: f32) -> (Vec2, Vec2, f32) {
        let (aim, rear) = (pose.transform_point(line.aim), pose.transform_point(line.rear));
        (on_screen(aim, fov), on_screen(rear, fov), (aim - rear).angle_between(Vec3::NEG_Z))
    }

    /// Aimed down the sights of this build, everything else still.
    fn aimed(def: &WeaponDef, parts: &[u8; SLOT_COUNT]) -> ViewState {
        ViewState { ads: 1.0, relief: eye_relief(&assemble(def, parts), &sight_line(def, parts)), ..default() }
    }

    #[test]
    fn every_sight_on_every_gun_is_on_the_screen_centre_when_aimed() {
        let mut worst = (0.0f32, String::new());
        let all = builds();
        assert!(all.len() > 3000, "{}", all.len());
        for (def, parts) in &all {
            let view = aimed(def, parts);
            let line = sight_line(def, parts);
            let fov = ads_fov(def, *parts);
            // At any moment: the idle breathing and wobble must not pull the sights off.
            for t in [0.0, 0.77, 1.9, 3.3, 5.1] {
                let pose = gun_pose(&view, &line, &Hold { t, ..default() }, Quat::IDENTITY);
                let (aim, rear, angle) = sight_picture(&pose, &line, fov);
                let what = format!("{} {:?} at t {t}", def.name, parts);
                assert!(aim.length() < 1.0, "{what}: the sights point {aim} px off the centre");
                assert!(rear.length() < 1.0, "{what}: the rear sight is {rear} px off the centre");
                assert!(angle < 2e-4, "{what}: sight line {angle} rad off the view direction");
                if aim.length() > worst.0 {
                    worst = (aim.length(), what);
                }
            }
            // Open space ahead never tucks the gun, however long the barrel.
            assert_eq!(wanted_tuck(resolve(def, &WeaponState { equipped: *parts, ..WeaponState::fresh(def) }, &standard_attachments()).length, None), 0.0, "{}", def.name);
        }
        assert!(worst.0 < 0.05, "worst {worst:?}");
    }

    /// Down the sights the gun reads as looking down it: the rear sight close to the eye,
    /// the barrel running straight away along the view (not canted or rolled), and no part of
    /// the gun sliced open by the camera's near plane.
    #[test]
    fn aiming_looks_straight_down_the_gun() {
        let mut reliefs: Vec<(f32, String)> = Vec::new();
        for (def, parts) in builds() {
            let view = aimed(&def, &parts);
            let line = sight_line(&def, &parts);
            let pose = gun_pose(&view, &line, &Hold::default(), Quat::IDENTITY);
            let what = format!("{} {:?}", def.name, parts);
            assert!((pose.rotation * Vec3::NEG_Z).angle_between(Vec3::NEG_Z) < 2e-4, "{what}: the barrel is not along the view");
            assert!((pose.rotation * Vec3::Y).angle_between(Vec3::Y) < 2e-4, "{what}: the gun is canted");
            let rear = pose.transform_point(line.rear);
            // As close as the gun's own stock allows, and never further out than the old fixed
            // spot (the grip 0.4 m ahead) left it.
            let old = 0.40 - line.rear.z * GUN_SCALE;
            assert!(-rear.z >= EYE_RELIEF[line.optic as usize] - 1e-4 && -rear.z <= old.max(EYE_RELIEF[line.optic as usize]) + 0.1, "{what}: rear sight {} m from the eye (was {old})", -rear.z);
            // No corner of the gun nearer than the near plane anywhere it could be seen.
            let model = assemble(&def, &parts);
            let tan = (BASE_FOV * 0.5).to_radians().tan();
            for p in model.parts.iter().flat_map(oz_core::model::part_points).chain(model.hulls.iter().flat_map(|h| h.points.clone())) {
                let q = pose.transform_point(p);
                let at = (-q.z).max(0.1);
                assert!(-q.z >= 0.1 || q.y.abs() > at * tan * 1.25 || q.x.abs() > at * tan * 2.6, "{what}: {q} is cut by the near plane");
            }
            reliefs.push((-rear.z, format!("{what} (was {old:.3})")));
        }
        reliefs.sort_by(|a, b| a.0.total_cmp(&b.0));
        if std::env::var("OZ_PRINT_RELIEF").is_ok() {
            for (relief, what) in reliefs.iter().filter(|r| r.1.contains("[0, 0, 0") || r.1.contains("[4, 0, 0")) {
                println!("relief {relief:.3} {what}");
            }
        }
    }

    #[test]
    fn a_wall_close_ahead_still_tucks_the_gun() {
        assert_eq!(wanted_tuck(0.9, Some(5.0)), 0.0);
        assert!(wanted_tuck(0.9, Some(0.5)) > 0.9);
        assert!(wanted_tuck(1.8, Some(1.2)) > 0.4, "a long gun tucks earlier");
    }

    /// Fire a burst down the sights and let it settle: the sights always point where the
    /// shots go (the camera's visual kick, shake and hold drift do not move them off the
    /// impact point), and once the gun's own kick has died away they are back on the centre.
    #[test]
    fn recoil_settles_back_onto_the_centre_and_the_sights_follow_the_shots() {
        let all = builds();
        let picks = [0usize, all.len() / 5, all.len() / 3, all.len() / 2, all.len() * 3 / 4, all.len() - 1];
        for &i in &picks {
            let (def, parts) = &all[i];
            let line = sight_line(def, parts);
            let fov = ads_fov(def, *parts);
            let mut view = ViewState { recoil: 2.5, kick: 2.5, kick_side: 0.04, kick_roll: -0.25, shake: 0.8, ..aimed(def, parts) };
            // Where the player aims, and so where the shots go.
            let look = Quat::from_euler(EulerRot::YXZ, 0.7, -0.1, 0.0);
            let dt = 1.0 / 60.0;
            for frame in 0..150 {
                let t = frame as f32 * dt;
                settle_kick(&mut view, dt);
                view.shake = (view.shake - dt * 5.0 * view.shake.max(0.25)).clamp(0.0, 2.5);
                // The camera as `follow_player` turns it: kicked up, shaken, drifting.
                let drift = Vec2::new((t * 0.8).sin(), (t * 1.3).cos() * 0.7) * 0.0035;
                let shake = Vec2::new((t * 61.0).sin(), (t * 53.0).cos()) * view.shake * 0.012;
                let camera = Quat::from_euler(EulerRot::YXZ, 0.7 + shake.x + drift.x, -0.1 + view.recoil * 0.012 + shake.y + drift.y, 0.0);
                let shots = camera.inverse() * (look * Vec3::NEG_Z);
                let aim = shot_in_view(camera, look * Vec3::NEG_Z);
                // With the gun's own kick taken out, the sights sit exactly on the shots' line.
                let still = ViewState { recoil: 0.0, kick_side: 0.0, kick_roll: 0.0, ..view };
                let pose = gun_pose(&still, &line, &Hold { t, ..default() }, aim);
                let (picture, _, _) = sight_picture(&pose, &line, fov);
                let impact = on_screen(shots * 100.0, fov);
                assert!(picture.distance(impact) < 1.0, "{} frame {frame}: sights at {picture}, shots land at {impact}", def.name);
                if frame == 149 {
                    // Settled: the gun's kick is gone, so the sights are on the shots even with the
                    // camera still drifting; and with the camera on the aim, on the centre.
                    assert!(view.recoil == 0.0 && view.kick_side.abs() < 1e-4 && view.kick_roll.abs() < 1e-4);
                    let (settled, _, _) = sight_picture(&gun_pose(&view, &line, &Hold { t, ..default() }, aim), &line, fov);
                    assert!(settled.distance(impact) < 1.0, "{}: settled at {settled} with the shots at {impact}", def.name);
                    let (centred, _, _) = sight_picture(&gun_pose(&view, &line, &Hold { t, ..default() }, shot_in_view(look, look * Vec3::NEG_Z)), &line, fov);
                    assert!(centred.length() < 1.0, "{}: settled at {centred}", def.name);
                }
            }
        }
    }
}
