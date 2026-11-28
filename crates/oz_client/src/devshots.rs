//! Developer test hooks driven by environment variables. None of this runs unless asked for.
//!
//! * `OPNIX_AUTOSTART=1` starts the match from the lobby straight away (host / offline).
//! * `OPNIX_KILLCAM_TEST=dir` checks the kill cam against the truth: while you are alive a
//!   second camera rides in the nearest enemy's real eye (their live position, stance, lean
//!   and aim) and saves what it sees as `dir/live_<t>.png`; when the kill cam plays, each
//!   replayed moment is saved as `dir/replay_<t>.png` with `t` on the same clock, and both
//!   are logged to `dir/views.csv` so the camera numbers can be compared too. Quits after
//!   the first kill cam.

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
        if std::env::var("OPNIX_POSE_CAM").is_ok() {
            app.add_systems(Update, pose_cam.after(crate::avatar::AnimateRigs).run_if(in_state(MatchPhase::Active)));
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
