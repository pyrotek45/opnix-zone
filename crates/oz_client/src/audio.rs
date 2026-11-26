//! Sound playback. Every sound is synthesised at startup by `oz_core::synth`.

use crate::{fx::PlaySfx, settings::Settings, view::WorldCamera};
use bevy::{audio::Volume, platform::collections::HashMap, prelude::*};
use oz_core::synth::{self, Sfx};
use oz_sim::prelude::*;

#[derive(Resource)]
struct SoundBank {
    clips: HashMap<Sfx, Handle<AudioSource>>,
    /// When each sound last played, to stop a horde from stacking one sample fifty times.
    last: HashMap<Sfx, f32>,
}

#[derive(Component)]
struct Ambience(Sfx);

pub struct AudioFxPlugin;

impl Plugin for AudioFxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(Update, (play, ambience));
    }
}

fn setup(mut commands: Commands, mut sources: ResMut<Assets<AudioSource>>) {
    let mut all: Vec<Sfx> = (0..9).map(Sfx::Shot).collect();
    all.extend([
        Sfx::Reload, Sfx::DryFire, Sfx::Hit, Sfx::Kill, Sfx::Explosion, Sfx::EnemyShot, Sfx::EnemyDeath, Sfx::Hurt, Sfx::UiMove, Sfx::UiAccept,
        Sfx::UiDeny, Sfx::Buy, Sfx::Build, Sfx::Mine, Sfx::OreGet, Sfx::Horn, Sfx::Dawn, Sfx::Alarm, Sfx::Step, Sfx::Land, Sfx::Swap, Sfx::Growl, Sfx::Surf, Sfx::NightDrone,
        Sfx::Headshot, Sfx::Streak, Sfx::Medal, Sfx::Jet, Sfx::Heartbeat, Sfx::Radar,
    ]);
    let clips: HashMap<Sfx, Handle<AudioSource>> = all.into_iter().map(|sfx| (sfx, sources.add(AudioSource { bytes: synth::wav(&synth::render(sfx)).into() }))).collect();
    for sfx in [Sfx::Surf, Sfx::NightDrone] {
        commands.spawn((Ambience(sfx), AudioPlayer::new(clips[&sfx].clone()), PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0))));
    }
    commands.insert_resource(SoundBank { clips, last: HashMap::default() });
}

fn play(
    mut events: EventReader<PlaySfx>,
    mut bank: ResMut<SoundBank>,
    mut commands: Commands,
    time: Res<Time<Real>>,
    settings: Res<Settings>,
    listener: Query<&Transform, With<WorldCamera>>,
    mut seed: Local<u32>,
) {
    let now = time.elapsed_secs();
    let ears = listener.single().ok().map(|t| t.translation);
    for event in events.read() {
        if bank.last.get(&event.sfx).is_some_and(|last| now - last < 0.04) {
            continue;
        }
        // Cheap distance falloff instead of full spatial audio.
        let distance = event.at.zip(ears).map_or(0.0, |(at, ears)| at.distance(ears));
        let gain = event.volume * settings.volume * (1.0 / (1.0 + distance * 0.06)).powi(2);
        if gain < 0.02 {
            continue;
        }
        let Some(clip) = bank.clips.get(&event.sfx).cloned() else { continue };
        bank.last.insert(event.sfx, now);
        // A little random pitch stops repeated sounds from feeling machine-stamped.
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let wobble = (*seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
        let musical = matches!(event.sfx, Sfx::UiMove | Sfx::UiAccept | Sfx::UiDeny | Sfx::Buy | Sfx::Dawn | Sfx::Horn | Sfx::Kill);
        let speed = if musical { 1.0 } else { 1.0 + wobble * 0.16 };
        commands.spawn((AudioPlayer::new(clip), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(gain)).with_speed(speed)));
    }
}

fn ambience(settings: Res<Settings>, state: Option<Res<MatchState>>, phase: Res<State<MatchPhase>>, mut loops: Query<(&Ambience, &mut AudioSink)>) {
    let playing = *phase.get() == MatchPhase::Active;
    let night = state.filter(|_| playing).map_or(0.0, |state| 1.0 - state.daylight);
    for (ambience, mut sink) in &mut loops {
        let level = match ambience.0 {
            Sfx::Surf => if playing { 0.3 } else { 0.12 },
            _ => night * 0.45,
        };
        sink.set_volume(Volume::Linear(level * settings.volume));
    }
}
