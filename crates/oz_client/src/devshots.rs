//! Developer test hooks driven by environment variables. None of this runs unless asked for.
//!
//! * `OPNIX_AUTOSTART=1` starts the match from the lobby straight away (host / offline).
//! * `OPNIX_KILLCAM_TEST=dir` checks the kill cam against the truth: while you are alive a
//!   second camera rides in the nearest enemy's real eye (their live position, stance, lean
//!   and aim) and saves what it sees as `dir/live_<t>.png`; when the kill cam plays, each
//!   replayed moment is saved as `dir/replay_<t>.png` with `t` on the same clock, and both
//!   are logged to `dir/views.csv` so the camera numbers can be compared too. Quits after
//!   the first kill cam.
//! * `OPNIX_ADS=weapon[:o,m,b,u,g,s,a]` holds the local player down the sights of that catalog
//!   weapon, fitted with those parts (slot variants), looking level (`OPNIX_ADS_PITCH` radians),
//!   to photograph the sight picture with `OPNIX_SHOTS`.

use crate::{
    avatar::{OwnRobot, PlayerRobot},
    killcam::KillCam,
    ui::Screen,
};
use bevy::{
    prelude::*,
    render::{
        camera::RenderTarget,
        render_asset::RenderAssetUsages,
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
};
use oz_sim::prelude::*;
use std::{collections::VecDeque, io::Write};

pub struct DevShotsPlugin;

impl Plugin for DevShotsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, autostart.run_if(in_state(MatchPhase::Active)));
        if std::env::var("OPNIX_UI_SHOT").is_ok() {
            app.add_systems(Update, ui_shot);
        }
        if std::env::var("OPNIX_POSE_CAM").is_ok() {
            app.add_systems(Update, pose_cam.after(crate::avatar::AnimateRigs).run_if(in_state(MatchPhase::Active)));
        }
        if std::env::var("OPNIX_ADS").is_ok() {
            app.add_systems(Update, hold_sights.in_set(crate::ClientSet::Input).after(crate::input::DrivePlayer).run_if(in_state(MatchPhase::Active)));
        }
        if std::env::var("OPNIX_KILLCAM_TEST").is_ok() {
            app.add_systems(Update, killcam_test.after(crate::avatar::AnimateRigs).run_if(in_state(MatchPhase::Active)));
        }
    }
}

fn autostart(screen: Res<Screen>, state: Res<MatchState>, players: Query<&Player, With<LocalPlayer>>, mut orders: EventWriter<CommandEvent>, mut asked: Local<f32>, time: Res<Time>) {
    if std::env::var("OPNIX_AUTOSTART").is_err() || state.phase != Phase::Lobby || *screen != Screen::Lobby || state.countdown > 0.0 {
        return;
    }
    let now = time.elapsed_secs();
    if now - *asked < 1.0 {
        return;
    }
    *asked = now;
    if let Ok(me) = players.single() {
        orders.write(CommandEvent { player: me.id, command: PlayerCommand::Lobby(LobbyAction::Start) });
    }
}

/// `OPNIX_UI_SHOT=path.png`: photographs a menu screen, then quits.
/// * `OPNIX_UI_SCREEN=join|settings|controls` opens that screen from the main menu first;
/// * otherwise, once in the lobby, `OPNIX_UI_LOBBY=mode-1,mapkind,size+1,map+1,newmap,copy,page1`
///   steps through those lobby actions (each once the lobby has settled on the last);
/// * the picture is taken `OPNIX_UI_DELAY` seconds (default 3) after the last step.
#[allow(clippy::too_many_arguments)]
fn ui_shot(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut screen: ResMut<Screen>,
    state: Option<Res<MatchState>>,
    players: Query<&Player, With<LocalPlayer>>,
    mut orders: EventWriter<CommandEvent>,
    mut acts: EventWriter<crate::ui::ActEvent>,
    mut exit: EventWriter<AppExit>,
    mut progress: Local<(usize, f32, bool)>,
) {
    let Ok(path) = std::env::var("OPNIX_UI_SHOT") else { return };
    let now = time.elapsed_secs();
    let step = progress.0;
    if progress.2 {
        if now - progress.1 > 2.0 {
            exit.write(AppExit::Success);
        }
        return;
    }
    let delay = std::env::var("OPNIX_UI_DELAY").ok().and_then(|d| d.parse().ok()).unwrap_or(3.0f32);
    let wanted = std::env::var("OPNIX_UI_SCREEN").ok();
    let ready = match wanted.as_deref() {
        Some(name) => {
            let target = match name {
                "join" => Screen::Join,
                "controls" => Screen::Controls,
                _ => Screen::Settings,
            };
            if *screen == Screen::Main && now > 3.0 {
                *screen = target.clone();
                progress.1 = now;
            }
            *screen == target
        }
        None => {
            let settled = state.as_ref().is_some_and(|s| s.phase == Phase::Lobby && s.change_in <= 0.0 && !s.reroll && s.countdown <= 0.0);
            if !(settled && *screen == Screen::Lobby) {
                progress.1 = now;
                return;
            }
            let steps: Vec<String> = std::env::var("OPNIX_UI_LOBBY").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(str::to_string).collect();
            if step < steps.len() {
                if now - progress.1 > 2.5
                    && let Ok(me) = players.single()
                {
                    let action = match steps[step].as_str() {
                        "mode-1" => Some(LobbyAction::Mode(-1)),
                        "mode+1" => Some(LobbyAction::Mode(1)),
                        "mapkind" => Some(LobbyAction::MapKind),
                        "size-1" => Some(LobbyAction::MapSize(-1)),
                        "size+1" => Some(LobbyAction::MapSize(1)),
                        "map+1" => Some(LobbyAction::Map(1)),
                        "map-1" => Some(LobbyAction::Map(-1)),
                        "newmap" => Some(LobbyAction::NewMap),
                        "copy" => {
                            acts.write(crate::ui::ActEvent(crate::ui::Act::CopyCode));
                            None
                        }
                        "page1" => {
                            acts.write(crate::ui::ActEvent(crate::ui::Act::LobbyPage(1)));
                            None
                        }
                        _ => None,
                    };
                    if let Some(action) = action {
                        orders.write(CommandEvent { player: me.id, command: PlayerCommand::Lobby(action) });
                    }
                    progress.0 += 1;
                    progress.1 = now;
                }
                return;
            }
            true
        }
    };
    if ready && now - progress.1 > delay {
        commands.spawn(Screenshot::primary_window()).observe(save_ppm(path));
        progress.2 = true;
        progress.1 = now;
    }
}

#[derive(Component)]
struct TruthCamera;

/// Saves a screenshot as a plain PPM (no image encoders needed); `path` ends in `.png` for
/// readability and is written as `.ppm` beside it.
pub fn save_ppm(path: String) -> impl FnMut(Trigger<ScreenshotCaptured>) {
    move |trigger| {
        let Ok(image) = trigger.event().0.clone().try_into_dynamic() else { return };
        let rgb = image.to_rgb8();
        let mut bytes = format!("P6\n{} {}\n255\n", rgb.width(), rgb.height()).into_bytes();
        bytes.extend_from_slice(rgb.as_raw());
        let _ = std::fs::write(path.replace(".png", ".ppm"), bytes);
    }
}

#[derive(Default)]
struct Test {
    image: Option<Handle<Image>>,
    last: f32,
    saved: VecDeque<(f32, String)>,
    played: bool,
    replay_last: f32,
    done_at: Option<f32>,
}

#[allow(clippy::too_many_arguments)]
fn killcam_test(
    mut commands: Commands,
    time: Res<Time>,
    state: Res<MatchState>,
    statics: Res<Statics>,
    cam: Res<KillCam>,
    mut images: ResMut<Assets<Image>>,
    mut test: Local<Test>,
    players: Query<(&Player, &Transform, &PlayerInput, &Health, Has<LocalPlayer>)>,
    mut robots: Query<(&Player, &mut Visibility, Has<OwnRobot>), (With<PlayerRobot>, Without<TruthCamera>)>,
    mut truth: Query<(&mut Transform, &mut Camera), (With<TruthCamera>, Without<Player>)>,
    world: Query<&GlobalTransform, With<crate::view::WorldCamera>>,
    mut exit: EventWriter<AppExit>,
) {
    let Ok(dir) = std::env::var("OPNIX_KILLCAM_TEST") else { return };
    let now = time.elapsed_secs();
    let log = |line: String| {
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(format!("{dir}/views.csv")) {
            let _ = writeln!(file, "{line}");
        }
    };
    if let Some(at) = test.done_at {
        if now > at {
            exit.write(AppExit::Success);
        }
        return;
    }
    let image = test
        .image
        .get_or_insert_with(|| {
            let size = Extent3d { width: 640, height: 360, depth_or_array_layers: 1 };
            let mut image = Image::new_fill(size, TextureDimension::D2, &[0, 0, 0, 255], TextureFormat::Bgra8UnormSrgb, RenderAssetUsages::default());
            image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
            let handle = images.add(image);
            commands.spawn((
                MatchEntity,
                TruthCamera,
                Camera3d::default(),
                Camera { target: RenderTarget::Image(handle.clone().into()), order: -5, clear_color: ClearColorConfig::Custom(Color::srgb(0.55, 0.6, 0.7)), ..default() },
                Projection::Perspective(PerspectiveProjection { fov: crate::view::BASE_FOV.to_radians(), near: 0.05, far: 600.0, ..default() }),
                Msaa::Off,
                bevy::core_pipeline::tonemapping::Tonemapping::Reinhard,
                Transform::default(),
            ));
            log("kind,t,x,y,z,yaw,pitch,victim_x,victim_y,victim_z".into());
            handle
        })
        .clone();
    let Some(me) = players.iter().find(|(.., local)| *local) else { return };
    let playing = cam.playing();
    let Ok((mut truth_at, mut truth_cam)) = truth.single_mut() else { return };
    truth_cam.is_active = !playing && state.phase == Phase::Live && me.3.alive();

    if playing {
        if !test.played
            && let Some((killer, death)) = cam.replay_subject()
        {
            for (t, p, yaw, pitch) in cam.track(killer).into_iter().filter(|(t, ..)| (*t - death).abs() < 4.0) {
                log(format!("recorded,{t:.3},{:.3},{:.3},{:.3},{yaw:.4},{pitch:.4},,,", p.x, p.y, p.z));
            }
            log(format!("death,{death:.3},,,,,,,,"));
        }
        test.played = true;
        let Some(clock) = cam.replay_clock() else { return };
        if clock - test.replay_last >= 0.1 {
            test.replay_last = clock;
            commands.spawn(Screenshot::primary_window()).observe(save_ppm(format!("{dir}/replay_{clock:08.2}.png")));
            if let Ok(eye) = world.single() {
                let (yaw, pitch, _) = eye.rotation().to_euler(EulerRot::YXZ);
                let p = eye.translation();
                let pov = cam.pov.map_or("none".to_string(), |v| format!("{:.3} {:.3} {:.3} yaw {:.4} pitch {:.4}", v.at.x, v.at.y, v.at.z, v.yaw, v.pitch));
                log(format!("replay,{clock:.3},{:.3},{:.3},{:.3},{yaw:.4},{pitch:.4},pov {pov},,", p.x, p.y, p.z));
            }
        }
        return;
    }
    if test.played {
        test.done_at = Some(now + 1.0);
        return;
    }
    if state.phase != Phase::Live || !me.3.alive() {
        return;
    }
    // The nearest living enemy: the one who will kill us in a 1v1.
    let (my_player, my_at) = (me.0, me.1.translation);
    let Some(enemy) = players.iter().filter(|(p, _, _, h, local)| !local && p.team != my_player.team && h.alive()).min_by(|a, b| a.1.translation.distance(my_at).total_cmp(&b.1.translation.distance(my_at))) else {
        return;
    };
    for (player, mut shown, own) in &mut robots {
        let wanted = if player.id == enemy.0.id { Visibility::Hidden } else if own { Visibility::Inherited } else { continue };
        if *shown != wanted {
            *shown = wanted;
        }
    }
    let eye = oz_sim::player::eye_point(&statics, enemy.1.translation, enemy.2);
    *truth_at = Transform::from_translation(eye).with_rotation(Quat::from_euler(EulerRot::YXZ, enemy.2.yaw, enemy.2.pitch, 0.0));
    if now - test.last >= 0.2 {
        test.last = now;
        let path = format!("{dir}/live_{now:08.2}.png");
        commands.spawn(Screenshot::image(image)).observe(save_ppm(path.clone()));
        log(format!("live,{now:.3},{:.3},{:.3},{:.3},{:.4},{:.4},{:.3},{:.3},{:.3}", eye.x, eye.y, eye.z, enemy.2.yaw, enemy.2.pitch, my_at.x, my_at.y, my_at.z));
        test.saved.push_back((now, path));
        // Only the last few seconds matter.
        while test.saved.front().is_some_and(|(t, _)| *t < now - 6.0) {
            if let Some((_, old)) = test.saved.pop_front() {
                let _ = std::fs::remove_file(old.replace(".png", ".ppm"));
            }
        }
    }
}

#[derive(Component)]
struct PoseCamera;

/// `OPNIX_POSE_CAM=dir`: a camera off to the right of your own robot (and a little
/// ahead, along the look) films it once a second as `dir/pose_<t>.ppm`, so its pose can be
/// judged side on. `OPNIX_POSE_SIDE=-1` films from the left.
fn pose_cam(
    mut commands: Commands,
    time: Res<Time>,
    mut images: ResMut<Assets<Image>>,
    mut image: Local<Option<Handle<Image>>>,
    mut last: Local<f32>,
    me: Query<(&Transform, &PlayerInput), With<LocalPlayer>>,
    mut cams: Query<&mut Transform, (With<PoseCamera>, Without<LocalPlayer>)>,
) {
    let Ok(dir) = std::env::var("OPNIX_POSE_CAM") else { return };
    let handle = image
        .get_or_insert_with(|| {
            let size = Extent3d { width: 640, height: 480, depth_or_array_layers: 1 };
            let mut target = Image::new_fill(size, TextureDimension::D2, &[0, 0, 0, 255], TextureFormat::Bgra8UnormSrgb, RenderAssetUsages::default());
            target.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
            let handle = images.add(target);
            commands.spawn((
                MatchEntity,
                PoseCamera,
                Camera3d::default(),
                Camera { target: RenderTarget::Image(handle.clone().into()), order: -6, clear_color: ClearColorConfig::Custom(Color::srgb(0.55, 0.6, 0.7)), ..default() },
                Projection::Perspective(PerspectiveProjection { fov: 50f32.to_radians(), near: 0.05, far: 300.0, ..default() }),
                Msaa::Off,
                bevy::core_pipeline::tonemapping::Tonemapping::Reinhard,
                Transform::default(),
            ));
            handle
        })
        .clone();
    let mut mine = me.iter();
    let count = mine.len();
    let Some((body, input)) = mine.next() else { return };
    if count != 1 {
        warn_once!("pose cam: {count} local players");
    }
    let side: f32 = std::env::var("OPNIX_POSE_SIDE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let look = Quat::from_rotation_y(input.yaw);
    let chest = body.translation + Vec3::Y * 1.0;
    let at = chest + look * Vec3::new(2.6 * side, 0.5, -0.8);
    if let Ok(mut cam) = cams.single_mut() {
        *cam = Transform::from_translation(at).looking_at(chest, Vec3::Y);
    }
    let now = time.elapsed_secs();
    if now - *last >= 1.0 {
        *last = now;
        info!("pose cam at {at} looking at {chest}");
        commands.spawn(Screenshot::image(handle)).observe(save_ppm(format!("{dir}/pose_{now:07.2}.png")));
    }
}

/// `OPNIX_ADS=weapon[:parts]`: aim down the sights of the given gun, kitted as asked.
fn hold_sights(mut controls: ResMut<crate::input::Controls>, mut players: Query<(&mut PlayerInput, &mut Arsenal), With<LocalPlayer>>) {
    let Ok(spec) = std::env::var("OPNIX_ADS") else { return };
    let Ok((mut input, mut arsenal)) = players.single_mut() else { return };
    let mut halves = spec.splitn(2, ':');
    let Some(weapon) = halves.next().and_then(|w| w.trim().parse::<usize>().ok()) else { return };
    let parts: Vec<u8> = halves.next().unwrap_or("").split(',').filter_map(|p| p.trim().parse().ok()).collect();
    let Some(state) = arsenal.weapons.get_mut(weapon) else { return };
    state.owned = true;
    for (slot, part) in parts.iter().enumerate().take(oz_core::arsenal::SLOT_COUNT) {
        state.equipped[slot] = *part;
    }
    arsenal.current = weapon as u8;
    input.weapon = weapon as u8;
    input.pitch = std::env::var("OPNIX_ADS_PITCH").ok().and_then(|p| p.parse().ok()).unwrap_or(0.0);
    input.buttons |= buttons::AIM;
    input.buttons &= !buttons::FIRE;
    controls.aim = true;
}
