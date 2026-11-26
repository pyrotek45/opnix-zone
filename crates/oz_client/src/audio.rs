//! Sound playback. Every sound is synthesised at startup by `oz_core::synth`.
//!
//! * **Spatial:** sounds with a position are panned between the ears and fade
//!   with distance (full volume within ~12 m, inverse-square beyond).
//! * **Acoustic space:** each positional sound picks a take rendered for where
//!   it happens - open air, a room (short reverb) or far away (dull, with a
//!   long tail). A wall between it and you muffles it further.
//! * **Variety:** busy sounds have several takes plus a little random pitch.
//! * **Ears:** a flashbang leaves everything muffled under a ringing tone.
//! * **Ambience:** surf, wind, rain (thunder now and then), birds by day, a
//!   machine drone at night - mixed by weather, time of day and whether you
//!   are indoors.
//! * **Sound packs:** any sound can be replaced by recordings. Put WAV files
//!   in a `sounds` folder (next to the game, in the working directory, or in
//!   the settings folder) named after the sound - `shot_3.wav`, `step.wav`,
//!   `step_wood.wav`, `explosion.wav`... - and add `_2`, `_3` for extra takes.
//!   See [`file_name`] for the names.

use crate::{fx::PlaySfx, settings::Settings, view::WorldCamera};
use bevy::{
    audio::{DefaultSpatialScale, SpatialScale, Volume},
    platform::collections::HashMap,
    prelude::*,
};
use oz_core::{
    synth::{self, Sfx, Space},
    worldgen::Weather,
};
use oz_sim::prelude::*;

#[derive(Resource)]
struct SoundBank {
    clips: HashMap<(Sfx, Space), Vec<Handle<AudioSource>>>,
    /// When each sound last played near a spot, to stop a horde stacking one sample fifty times.
    last: HashMap<(Sfx, i32, i32), f32>,
}

#[derive(Component)]
struct Ambience(Sfx);

/// A looping sound attached to a moving thing (a rocket's motor, a drone's rotors).
#[derive(Component)]
pub struct Engine;

pub struct AudioFxPlugin;

impl Plugin for AudioFxPlugin {
    fn build(&self, app: &mut App) {
        // Full volume within about 12 m, then inverse-square falloff.
        app.insert_resource(DefaultSpatialScale(SpatialScale::new(1.0 / 12.0)))
            .add_systems(Startup, setup)
            .add_systems(Update, (play, ambience, thunder, engines, listen));
    }
}

/// The file name a sound pack uses for a sound: `Shot(3)` -> `shot_3`, `StepWood` -> `step_wood`.
pub fn file_name(sfx: Sfx) -> String {
    let debug = format!("{sfx:?}").replace(['(', ')'], "_");
    let mut name = String::new();
    for (i, c) in debug.trim_end_matches('_').chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 && !name.ends_with('_') {
            name.push('_');
        }
        name.push(c.to_ascii_lowercase());
    }
    name
}

/// Folders searched for sound-pack WAV files.
fn pack_folders() -> Vec<std::path::PathBuf> {
    let mut folders = vec![std::path::PathBuf::from("sounds")];
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|p| p.join("sounds"))) {
        folders.push(dir);
    }
    if let Some(dir) = crate::settings::config_dir() {
        folders.push(dir.join("sounds"));
    }
    folders
}

/// Recorded takes for a sound from a sound pack, if any.
fn pack_takes(folders: &[std::path::PathBuf], sfx: Sfx) -> Vec<Vec<u8>> {
    let name = file_name(sfx);
    let mut takes = Vec::new();
    for folder in folders {
        for suffix in std::iter::once(String::new()).chain((1..10).map(|n| format!("_{n}"))) {
            if let Ok(bytes) = std::fs::read(folder.join(format!("{name}{suffix}.wav"))) {
                takes.push(bytes);
            }
        }
        if !takes.is_empty() {
            break;
        }
    }
    takes
}

fn setup(mut commands: Commands, mut sources: ResMut<Assets<AudioSource>>) {
    let mut clips: HashMap<(Sfx, Space), Vec<Handle<AudioSource>>> = HashMap::default();
    let folders = pack_folders();
    for sfx in Sfx::all() {
        // Recordings from a sound pack win over the synthesiser.
        let recorded = pack_takes(&folders, sfx);
        if !recorded.is_empty() {
            info!("sound pack: {} take(s) for {}", recorded.len(), file_name(sfx));
            let takes: Vec<Handle<AudioSource>> = recorded.into_iter().map(|bytes| sources.add(AudioSource { bytes: bytes.into() })).collect();
            for space in [Space::Open, Space::Room, Space::Far] {
                clips.insert((sfx, space), takes.clone());
            }
            continue;
        }
        let spaces: &[Space] = if sfx.spatial_variants() { &[Space::Open, Space::Room, Space::Far] } else { &[Space::Open] };
        for space in spaces {
            let takes = (0..sfx.variants()).map(|take| sources.add(AudioSource { bytes: synth::wav(&synth::render_in(sfx, take, *space)).into() })).collect();
            clips.insert((sfx, *space), takes);
        }
    }
    for sfx in [Sfx::Surf, Sfx::NightDrone, Sfx::Rain, Sfx::Wind, Sfx::Birds] {
        commands.spawn((Ambience(sfx), AudioPlayer::new(clips[&(sfx, Space::Open)][0].clone()), PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0))));
    }
    commands.insert_resource(SoundBank { clips, last: HashMap::default() });
}

/// The camera hears: give it ears.
fn listen(mut commands: Commands, cameras: Query<Entity, (With<WorldCamera>, Without<SpatialListener>)>) {
    for camera in &cameras {
        commands.entity(camera).insert(SpatialListener::new(0.35));
    }
}

fn footstep(sfx: Sfx) -> bool {
    matches!(sfx, Sfx::Step | Sfx::StepSoft | Sfx::StepHeavy | Sfx::StepWood | Sfx::Crawl | Sfx::Land | Sfx::Jump)
}

/// Is there a roof over this point?
fn indoors(statics: &Statics, at: Vec3) -> bool {
    statics.raycast(at + Vec3::Y * 0.3, Vec3::Y, 9.0).is_some()
}

#[allow(clippy::too_many_arguments)]
fn play(
    mut events: EventReader<PlaySfx>,
    mut bank: ResMut<SoundBank>,
    mut commands: Commands,
    time: Res<Time<Real>>,
    settings: Res<Settings>,
    statics: Option<Res<Statics>>,
    view: Option<Res<crate::view::ViewState>>,
    listener: Query<&Transform, With<WorldCamera>>,
    mut waiting: Local<Vec<(f32, PlaySfx)>>,
    mut seed: Local<u32>,
) {
    let now = time.elapsed_secs();
    let ears = listener.single().ok().map(|t| t.translation);
    // Delayed sounds join the queue until they are due.
    for event in events.read() {
        waiting.push((now + event.delay, *event));
    }
    let (due, later): (Vec<_>, Vec<_>) = waiting.drain(..).partition(|(at, _)| *at <= now);
    *waiting = later;
    // A flashbang leaves the world muffled for a while.
    let deafened = view.as_ref().map_or(0.0, |v| v.blind.min(1.0));
    let ears_inside = ears.zip(statics.as_deref()).is_some_and(|(at, s)| indoors(s, at - Vec3::Y * 1.5));

    for (_, event) in due {
        let distance = event.at.zip(ears).map_or(0.0, |(at, ears)| at.distance(ears));
        if distance > 160.0 {
            continue;
        }
        let bucket = event.at.map_or((0, 0), |at| ((at.x / 4.0) as i32, (at.z / 4.0) as i32));
        let key = (event.sfx, bucket.0, bucket.1);
        if bank.last.get(&key).is_some_and(|last| now - last < 0.035) {
            continue;
        }
        // Where is it heard? Far away, behind a wall, in a room or in the open.
        let occluded = match (event.at, ears, statics.as_deref()) {
            (Some(at), Some(ears), Some(statics)) if distance > 2.0 => {
                let span = at + Vec3::Y * 0.6 - ears;
                statics.raycast(ears, span / distance.max(1e-3), distance - 0.8).is_some()
            }
            _ => false,
        };
        let space = if !event.sfx.spatial_variants() {
            Space::Open
        } else if distance > 45.0 || (occluded && distance > 15.0) {
            Space::Far
        } else {
            let inside = match (event.at, statics.as_deref()) {
                (Some(at), Some(statics)) => indoors(statics, at),
                _ => ears_inside,
            };
            if inside { Space::Room } else { Space::Open }
        };
        let Some(takes) = bank.clips.get(&(event.sfx, space)).or_else(|| bank.clips.get(&(event.sfx, Space::Open))) else { continue };
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let clip = takes[(*seed >> 16) as usize % takes.len()].clone();
        bank.last.insert(key, now);

        let muffle = if occluded { 0.55 } else { 1.0 };
        // Nothing plays at exactly the same level twice.
        let musical = matches!(event.sfx, Sfx::UiMove | Sfx::UiAccept | Sfx::UiDeny | Sfx::Buy | Sfx::Dawn | Sfx::Horn | Sfx::Kill | Sfx::Streak | Sfx::Medal | Sfx::Headshot);
        let jitter = ((*seed >> 4) & 0xfff) as f32 / 4096.0 - 0.5;
        let level = if musical { 1.0 } else { 1.0 + jitter * if footstep(event.sfx) { 0.5 } else { 0.25 } };
        // 0.7 of headroom so a firefight's many layers do not clip when they stack.
        let gain = 0.7 * event.volume * level * settings.volume * muffle * (1.0 - deafened * 0.85);
        if gain < 0.01 {
            continue;
        }
        // A little random pitch stops repeated sounds from feeling machine-stamped.
        let wobble = (*seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
        let speed = if musical { 1.0 } else { 1.0 + wobble * if footstep(event.sfx) { 0.22 } else { 0.12 } };
        let playback = PlaybackSettings::DESPAWN.with_volume(Volume::Linear(gain)).with_speed(speed);
        match event.at {
            Some(at) if distance > 0.5 => {
                commands.spawn((AudioPlayer::new(clip), playback.with_spatial(true), Transform::from_translation(at)));
            }
            _ => {
                commands.spawn((AudioPlayer::new(clip), playback));
            }
        }
    }
    bank.last.retain(|_, at| now - *at < 1.0);
}

#[allow(clippy::too_many_arguments)]
fn ambience(
    time: Res<Time<Real>>,
    settings: Res<Settings>,
    state: Option<Res<MatchState>>,
    rules: Option<Res<Rules>>,
    statics: Option<Res<Statics>>,
    phase: Res<State<MatchPhase>>,
    view: Option<Res<crate::view::ViewState>>,
    listener: Query<&Transform, With<WorldCamera>>,
    mut loops: Query<(&Ambience, &mut AudioSink)>,
    mut levels: Local<HashMap<Sfx, f32>>,
) {
    let playing = *phase.get() == MatchPhase::Active;
    let day = state.filter(|_| playing).map_or(1.0, |state| state.daylight);
    let weather = rules.filter(|_| playing).map_or(Weather::Clear, |r| r.recipe.weather);
    let inside = listener.single().ok().zip(statics.as_deref()).is_some_and(|(t, s)| indoors(s, t.translation - Vec3::Y * 1.5));
    let deafened = view.map_or(0.0, |v| v.blind.min(1.0));
    let roof = if inside { 0.35 } else { 1.0 };
    let dt = time.delta_secs();
    for (ambience, mut sink) in &mut loops {
        let wanted = match ambience.0 {
            Sfx::Surf => if playing { 0.22 * roof } else { 0.1 },
            Sfx::NightDrone => (1.0 - day) * 0.4,
            Sfx::Rain => if weather == Weather::Rain { 0.55 * roof.max(0.5) } else { 0.0 },
            Sfx::Wind => match weather {
                Weather::Rain | Weather::Overcast => 0.35 * roof,
                Weather::Fog => 0.22 * roof,
                Weather::Clear => if playing { 0.1 * roof } else { 0.0 },
            },
            Sfx::Birds if weather == Weather::Clear && playing => day * 0.3 * roof,
            _ => 0.0,
        } * (1.0 - deafened * 0.8);
        // Glide between levels so walking indoors softens the world instead of cutting it.
        let level = levels.entry(ambience.0).or_insert(0.0);
        *level += (wanted - *level) * (1.0 - (-dt * 2.0).exp());
        sink.set_volume(Volume::Linear(*level * settings.volume));
    }
}

/// Thunder rolls now and then while it rains.
fn thunder(time: Res<Time>, rules: Option<Res<Rules>>, phase: Res<State<MatchPhase>>, mut wait: Local<f32>, mut sounds: EventWriter<PlaySfx>, mut seed: Local<u32>) {
    if *phase.get() != MatchPhase::Active || rules.is_none_or(|r| r.recipe.weather != Weather::Rain) {
        return;
    }
    *wait -= time.delta_secs();
    if *wait <= 0.0 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *wait = 14.0 + (*seed >> 20) as f32 % 22.0;
        sounds.write(PlaySfx::new(Sfx::Thunder, None).quiet(0.7));
    }
}

/// Rockets and drones carry their own looping motors.
fn engines(mut commands: Commands, bank: Res<SoundBank>, settings: Res<Settings>, shots: Query<(Entity, &Projectile), (Added<Projectile>, Without<Engine>)>) {
    for (entity, shot) in &shots {
        let sfx = match shot.look {
            oz_core::content::ShotLook::Rocket => Sfx::RocketFly,
            oz_core::content::ShotLook::Drone => Sfx::DroneBuzz,
            _ => continue,
        };
        let Some(clip) = bank.clips.get(&(sfx, Space::Open)).and_then(|t| t.first()).cloned() else { continue };
        commands.entity(entity).insert(Engine).with_child((AudioPlayer::new(clip), PlaybackSettings::LOOP.with_volume(Volume::Linear(0.5 * settings.volume)).with_spatial(true), Transform::IDENTITY));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_pack_names() {
        assert_eq!(file_name(Sfx::Shot(3)), "shot_3");
        assert_eq!(file_name(Sfx::StepWood), "step_wood");
        assert_eq!(file_name(Sfx::Explosion), "explosion");
        assert_eq!(file_name(Sfx::ImpactConcrete), "impact_concrete");
    }
}
