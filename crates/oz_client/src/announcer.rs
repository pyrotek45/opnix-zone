//! The announcer's voice, if this machine has one.
//!
//! Nothing ships with the game - it is all code, no recordings - so the voice is
//! optional: install [Piper](https://github.com/rhasspy/piper) and a voice model and
//! the announcer reads out callouts ("Enemy radar plane spotted", "Incoming
//! artillery", "Mission complete"...). It is found by:
//!
//! * the `piper` program: `OPNIX_PIPER=/path/to/piper`, or `piper` on the `PATH`;
//! * a voice model: `OPNIX_PIPER_MODEL=/path/to/voice.onnx`, or the first `.onnx`
//!   file in the game's config folder under `voice/`.
//!
//! Lines are rendered on a background thread the first time they are needed (the
//! common ones while the game starts) and kept, so later calls play at once.
//! Without Piper the callouts still show on screen and the alarm still sounds.

use crate::{juice::Announce, settings::Settings};
use bevy::{audio::Volume, platform::collections::HashMap, prelude::*};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        mpsc::{Receiver, Sender, channel},
    },
};

/// Lines worth rendering before anyone needs them.
const COMMON: [&str; 14] = [
    "Enemy radar plane spotted.",
    "Enemy bomber inbound.",
    "Incoming artillery. Get indoors.",
    "Enemy attack drone. Shoot it down.",
    "Enemy supply drop.",
    "Radar plane is up.",
    "Airstrike inbound.",
    "Artillery on the way.",
    "Supply drop. Rearm.",
    "Attack drone deployed.",
    "Mission complete.",
    "Mission failed.",
    "Clear the map.",
    "Fight.",
];

#[derive(Resource)]
struct Voice {
    clips: HashMap<String, Handle<AudioSource>>,
    /// Lines asked for that are still being rendered, and when they were asked.
    waiting: HashMap<String, f64>,
    requests: Option<Sender<String>>,
    results: Mutex<Receiver<(String, Vec<u8>)>>,
}

pub struct AnnouncerPlugin;

impl Plugin for AnnouncerPlugin {
    fn build(&self, app: &mut App) {
        let (requests, results) = start();
        app.insert_resource(Voice { clips: HashMap::default(), waiting: HashMap::default(), requests, results: Mutex::new(results) }).add_systems(Update, speak);
    }
}

fn find_program() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OPNIX_PIPER").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(path);
    }
    let names: &[&str] = if cfg!(windows) { &["piper.exe"] } else { &["piper"] };
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).flat_map(|dir| names.iter().map(move |n| dir.join(n))).find(|p| p.is_file()))
}

fn find_model() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OPNIX_PIPER_MODEL").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(path);
    }
    let dir = crate::settings::config_dir()?.join("voice");
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "onnx"))
}

/// One line through Piper, as WAV bytes.
fn render(program: &Path, model: &Path, line: &str, scratch: &Path) -> Option<Vec<u8>> {
    use std::io::Write;
    let out = scratch.join(format!("line-{}.wav", line.len() ^ line.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize))));
    let mut child = std::process::Command::new(program)
        .arg("--model")
        .arg(model)
        .arg("--output_file")
        .arg(&out)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(line.as_bytes()).ok()?;
    child.wait().ok()?.success().then_some(())?;
    let bytes = std::fs::read(&out).ok();
    let _ = std::fs::remove_file(&out);
    bytes
}

/// Starts the renderer thread if a voice is installed.
fn start() -> (Option<Sender<String>>, Receiver<(String, Vec<u8>)>) {
    let (done, results) = channel();
    let (Some(program), Some(model)) = (find_program(), find_model()) else { return (None, results) };
    let (requests, incoming) = channel::<String>();
    let scratch = std::env::temp_dir().join("opnix-voice");
    let _ = std::fs::create_dir_all(&scratch);
    std::thread::spawn(move || {
        for line in COMMON.iter().map(|l| l.to_string()).chain(incoming.iter()) {
            if let Some(bytes) = render(&program, &model, &line, &scratch)
                && done.send((line, bytes)).is_err()
            {
                return;
            }
        }
    });
    (Some(requests), results)
}

fn speak(time: Res<Time<Real>>, mut voice: ResMut<Voice>, mut lines: EventReader<Announce>, mut sources: ResMut<Assets<AudioSource>>, settings: Res<Settings>, mut commands: Commands) {
    let now = time.elapsed_secs_f64();
    let play = |commands: &mut Commands, clip: Handle<AudioSource>| {
        commands.spawn((AudioPlayer::new(clip), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(settings.volume))));
    };
    // Finished renders: keep them, and say any that were asked for a moment ago.
    let finished: Vec<(String, Vec<u8>)> = voice.results.lock().map(|r| r.try_iter().collect()).unwrap_or_default();
    for (line, bytes) in finished {
        let clip = sources.add(AudioSource { bytes: bytes.into() });
        if voice.waiting.remove(&line).is_some_and(|asked| now - asked < 2.5) {
            play(&mut commands, clip.clone());
        }
        voice.clips.insert(line, clip);
    }
    for Announce(line) in lines.read() {
        if voice.requests.is_none() {
            continue;
        }
        if let Some(clip) = voice.clips.get(line) {
            play(&mut commands, clip.clone());
        } else if !voice.waiting.contains_key(line) {
            voice.waiting.insert(line.clone(), now);
            let asked = voice.requests.as_ref().is_some_and(|r| r.send(line.clone()).is_ok());
            if !asked {
                voice.requests = None;
            }
        }
    }
}
