//! A debug log the player can send back when something goes wrong.
//!
//! Every run writes `debug.log` in the game's config folder (the previous run's log is kept
//! as `debug.old.log`):
//!
//! * a header: build, launch arguments, operating system, graphics adapter, settings;
//! * each match: mode, map, seed and rules;
//! * warnings and errors from the engine and the game;
//! * kills, deaths and kill cams as they happen;
//! * twice a second, a one-line status (frame rate, your position, stance, health, gun, ammo,
//!   cover, camera) kept in memory for the last 20 seconds.
//!
//! Press **F8** the moment something looks wrong: an `ISSUE #n` block goes into the log
//! with the last 20 seconds of status lines, a full snapshot of every player and bot (where
//! they are, what they are doing, what they are aiming at) and the frame-time history. A
//! note on screen confirms it and says where the file is. Then describe what you saw
//! alongside the log.

use crate::{cover::Cover, killcam::KillCam, view::ViewState, view::WorldCamera};
use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    log::{BoxedLayer, tracing_subscriber::Layer},
    prelude::*,
};
use oz_sim::prelude::*;
use std::{
    collections::VecDeque,
    io::Write,
    path::PathBuf,
    sync::Mutex,
};

/// The open log file, shared with the engine's log layer.
static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
/// Seconds of status history kept for an issue report.
const HISTORY: f32 = 20.0;

/// Where this run's log is written.
pub fn log_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|dir| dir.join("debug.log"))
}

fn open() {
    let Ok(mut file) = FILE.lock() else { return };
    if file.is_some() {
        return;
    }
    let Some(path) = log_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::rename(&path, path.with_file_name("debug.old.log"));
    *file = std::fs::File::create(&path).ok();
}

/// Appends lines to the log.
pub fn write(text: &str) {
    open();
    if let Ok(mut file) = FILE.lock()
        && let Some(file) = file.as_mut()
    {
        let _ = writeln!(file, "{text}");
        let _ = file.flush();
    }
}

struct LogWriter;

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        open();
        if let Ok(mut file) = FILE.lock()
            && let Some(file) = file.as_mut()
        {
            file.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Ok(mut file) = FILE.lock()
            && let Some(file) = file.as_mut()
        {
            file.flush()?;
        }
        Ok(())
    }
}

/// For `LogPlugin::custom_layer`: engine and game warnings and errors (and the game's own
/// info lines) also go to the debug log.
pub fn log_layer(_app: &mut App) -> Option<BoxedLayer> {
    let layer = bevy::log::tracing_subscriber::fmt::Layer::default().with_ansi(false).with_target(true).with_writer(|| LogWriter).with_filter(
        bevy::log::tracing_subscriber::filter::FilterFn::new(|meta| {
            let target = meta.target();
            // A known harmless warning that would otherwise flood the file.
            if target.starts_with("bevy_ecs::hierarchy") {
                return false;
            }
            let ours = target.starts_with("oz_") || target.starts_with("iron_town") || target.starts_with("ember_atoll");
            *meta.level() <= bevy::log::Level::WARN || (ours && *meta.level() <= bevy::log::Level::INFO)
        }),
    );
    Some(Box::new(layer))
}

pub struct DebugLogPlugin;

impl Plugin for DebugLogPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }
        app.init_resource::<Status>().add_systems(Startup, header).add_systems(Update, (match_header, events, status, mark_issue, fade_note).chain().after(crate::avatar::AnimateRigs));
    }
}

#[derive(Resource, Default)]
struct Status {
    lines: VecDeque<(f32, String)>,
    frames: VecDeque<f32>,
    last: f32,
    issues: u32,
    worst: f32,
    rules: Option<Rules>,
}

#[derive(Component)]
struct IssueNote(f32);

fn header(settings: Res<crate::settings::Settings>, adapter: Option<Res<bevy::render::renderer::RenderAdapterInfo>>, title: Res<crate::GameTitle>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    write(&format!("=== {} debug log ===", title.0));
    write(&format!("build {} ({}), {} {}", env!("CARGO_PKG_VERSION"), if cfg!(debug_assertions) { "debug" } else { "release" }, std::env::consts::OS, std::env::consts::ARCH));
    write(&format!("started {:?} since unix epoch, args {args:?}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)));
    if let Some(adapter) = adapter {
        write(&format!("gpu {} ({:?}, {:?}, driver {} {})", adapter.name, adapter.device_type, adapter.backend, adapter.driver, adapter.driver_info));
    }
    write(&format!("settings fov {:.0} pixel {} vsync {} sensitivity {:.2}", settings.fov, settings.pixel_size, settings.vsync, settings.sensitivity));
    write("F8 in game marks an issue in this file.");
}

fn match_header(rules: Option<Res<Rules>>, role: Option<Res<NetRole>>, mut status: ResMut<Status>) {
    let Some(rules) = rules else { return };
    if status.rules.as_ref() == Some(&*rules) {
        return;
    }
    status.rules = Some(rules.clone());
    write(&format!("--- match: role {:?}, mode {}, team size {}, map {:?} ---", role.map(|r| *r), rules.mode, rules.team_size, rules.recipe));
    write(&format!("rules {rules:?}"));
}

fn events(time: Res<Time>, mut fx: EventReader<FxEvent>, players: Query<(&Player, &Transform)>, cam: Res<KillCam>, mut was_playing: Local<bool>) {
    let now = time.elapsed_secs();
    let name = |id: u8| players.iter().find(|(p, _)| p.id == id).map_or(format!("#{id}"), |(p, _)| format!("{} (#{id})", p.name));
    let at = |id: u8| players.iter().find(|(p, _)| p.id == id).map(|(_, t)| t.translation);
    for event in fx.read() {
        if let FxEvent::Killed { victim, by, head, blast } = event {
            let distance = at(*victim).zip(at(*by)).map_or(0.0, |(a, b)| a.distance(b));
            write(&format!("[{now:8.2}] KILL {} -> {} head {head} blast {blast} at {distance:.1} m (victim at {:?})", name(*by), name(*victim), at(*victim).map(|p| p.to_array().map(|v| (v * 10.0).round() / 10.0))));
        }
    }
    let playing = cam.playing();
    if playing != *was_playing {
        *was_playing = playing;
        write(&format!("[{now:8.2}] kill cam {}{}", if playing { "starts, replaying from " } else { "ends" }, cam.replay_clock().map_or(String::new(), |c| format!("{c:.2}"))));
    }
}

fn round(v: Vec3) -> String {
    format!("({:.2}, {:.2}, {:.2})", v.x, v.y, v.z)
}

#[allow(clippy::too_many_arguments)]
fn status(
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    state: Option<Res<MatchState>>,
    view: Res<ViewState>,
    cover: Res<Cover>,
    content: Option<Res<Content>>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
    me: Query<(&Player, &Transform, &PlayerInput, &Health, Option<&Arsenal>), With<LocalPlayer>>,
    mut status: ResMut<Status>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    status.frames.push_back(dt * 1000.0);
    while status.frames.len() > 600 {
        status.frames.pop_front();
    }
    status.worst = status.worst.max(dt);
    if now - status.last < 0.5 {
        return;
    }
    status.last = now;
    let fps = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed()).unwrap_or(0.0);
    let mut line = format!("[{now:8.2}] fps {fps:5.1} worst {:4.0} ms", status.worst * 1000.0);
    status.worst = 0.0;
    if let Some(state) = &state {
        line += &format!(" | {:?}", state.phase);
    }
    if let Ok((player, transform, input, health, arsenal)) = me.single() {
        let gun = arsenal.and_then(|a| {
            let w = a.weapons.get(a.current as usize)?;
            let name = content.as_ref().and_then(|c| c.weapons.get(a.current as usize)).map_or("?", |d| d.name);
            Some(format!("{name} {}/{}", w.ammo, w.reserve))
        });
        line += &format!(
            " | you #{} at {} yaw {:.2} pitch {:.2} stance {} lean {} hp {:.0} {} buttons {:#x}",
            player.id,
            round(transform.translation),
            input.yaw,
            input.pitch,
            input.stance,
            input.lean,
            health.hp,
            gun.unwrap_or_default(),
            input.buttons
        );
    }
    if cover.wall.is_some() {
        line += &format!(" | cover low {} aiming {}", cover.wall.is_some_and(|w| w.low), cover.aiming);
    }
    if let Ok(eye) = camera.single() {
        line += &format!(" | cam {} third {:.1}", round(eye.translation()), view.third);
    }
    status.lines.push_back((now, line));
    while status.lines.front().is_some_and(|(t, _)| *t < now - HISTORY) {
        status.lines.pop_front();
    }
}

#[allow(clippy::too_many_arguments)]
fn mark_issue(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    state: Option<Res<MatchState>>,
    content: Option<Res<Content>>,
    cam: Res<KillCam>,
    players: Query<(&Player, &Transform, &PlayerInput, &Health, Option<&Arsenal>, Option<&oz_sim::bots::Bot>, Has<LocalPlayer>)>,
    mut status: ResMut<Status>,
) {
    if !keys.just_pressed(KeyCode::F8) {
        return;
    }
    let now = time.elapsed_secs();
    status.issues += 1;
    let n = status.issues;
    let mut out = vec![format!("\n########## ISSUE #{n} marked at {now:.2} ##########"), "-- last 20 s of status --".to_string()];
    out.extend(status.lines.iter().map(|(_, l)| l.clone()));
    if let Some(state) = &state {
        out.push(format!("-- match: phase {:?} scores {:?} --", state.phase, state.scores));
    }
    out.push(format!("-- kill cam: playing {} clock {:?} --", cam.playing(), cam.replay_clock()));
    out.push("-- players --".to_string());
    for (player, transform, input, health, arsenal, bot, local) in &players {
        let gun = arsenal.and_then(|a| {
            let w = a.weapons.get(a.current as usize)?;
            let name = content.as_ref().and_then(|c| c.weapons.get(a.current as usize)).map_or("?", |d| d.name);
            Some(format!("{name} {}/{} reload {:.1}", w.ammo, w.reserve, a.reload))
        });
        let who = if local { "YOU".to_string() } else if let Some(bot) = bot { format!("bot {:?}/{:?}", bot.role, bot.plan) } else { "human".to_string() };
        out.push(format!(
            "#{} {} [{}] team {} hp {:.0} at {} yaw {:.2} pitch {:.2} stance {} lean {} move ({:.2},{:.2}) buttons {:#x} flags {:#x} {}",
            player.id,
            player.name,
            who,
            player.team,
            health.hp,
            round(transform.translation),
            input.yaw,
            input.pitch,
            input.stance,
            input.lean,
            input.movement.x,
            input.movement.y,
            input.buttons,
            player.flags,
            gun.unwrap_or_default()
        ));
    }
    let frames: Vec<String> = status.frames.iter().rev().take(120).map(|ms| format!("{ms:.0}")).collect();
    out.push(format!("-- last {} frame times (ms, newest first) --", frames.len()));
    out.push(frames.join(" "));
    out.push(format!("########## end of issue #{n} ##########\n"));
    write(&out.join("\n"));
    let path = log_path().map_or("debug.log".to_string(), |p| p.display().to_string());
    commands.spawn((
        IssueNote(4.0),
        Text::new(format!("ISSUE #{n} MARKED - saved to {path}")),
        TextFont { font_size: 16.0, ..default() },
        TextColor(Color::srgb(1.0, 0.85, 0.3)),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        Node { position_type: PositionType::Absolute, top: Val::Px(12.0), left: Val::Px(12.0), padding: UiRect::all(Val::Px(6.0)), ..default() },
        GlobalZIndex(90),
    ));
}

fn fade_note(mut commands: Commands, time: Res<Time>, mut notes: Query<(Entity, &mut IssueNote)>) {
    for (entity, mut note) in &mut notes {
        note.0 -= time.delta_secs();
        if note.0 <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}
