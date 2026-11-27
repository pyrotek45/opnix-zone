//! Persistent player options, stored as `key=value` lines in the user's config folder.

use bevy::prelude::*;
use std::{fs, path::PathBuf};

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Settings {
    pub sensitivity: f32,
    /// Look speed multiplier while aiming down sights.
    pub ads_sensitivity: f32,
    /// Horizontal-ish field of view in degrees.
    pub fov: f32,
    /// Show frame rate and ping in the corner.
    pub net_stats: bool,
    pub invert_y: bool,
    /// Size of one rendered pixel on screen. Higher = chunkier and much cheaper to render.
    pub pixel_size: u32,
    pub dither: bool,
    pub shadows: bool,
    pub fullscreen: bool,
    pub vsync: bool,
    pub volume: f32,
    /// Combat effects: full (hit-stop, big shakes, every particle) or reduced (calmer, cheaper).
    pub full_effects: bool,
    /// Controller aim assist (slowdown over targets and a gentle pull while moving).
    pub aim_assist: bool,
    /// Right-stick response: dynamic (fine near the centre, quick at the edge) or standard.
    pub dynamic_curve: bool,
    pub callsign: String,
    pub last_address: String,
    pub bindings: crate::bindings::Bindings,
    /// Identifies this install to hosts, so a dropped connection can take its seat back.
    pub token: u64,
    /// The five saved classes, encoded by `loadout` (empty = defaults).
    pub classes: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sensitivity: 1.0,
            ads_sensitivity: 0.8,
            fov: 75.0,
            net_stats: false,
            invert_y: false,
            pixel_size: 3,
            dither: true,
            shadows: true,
            fullscreen: false,
            vsync: true,
            volume: 0.8,
            full_effects: true,
            aim_assist: true,
            dynamic_curve: true,
            callsign: "OPERATOR".into(),
            last_address: "127.0.0.1".into(),
            bindings: Default::default(),
            token: 0,
            classes: Vec::new(),
        }
    }
}

/// The game's folder in the user's config directory.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .map(|dir| dir.join("opnix-zone"))
}

fn path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("settings.txt"))
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Self::default();
        let Some(text) = path().and_then(|p| fs::read_to_string(p).ok()) else {
            s.ensure_token();
            return s;
        };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let number = value.parse::<f32>().ok().filter(|n| n.is_finite());
            let flag = value == "true";
            match key {
                "sensitivity" => s.sensitivity = number.unwrap_or(1.0).clamp(0.2, 3.0),
                "invert_y" => s.invert_y = flag,
                "ads_sensitivity" => s.ads_sensitivity = number.unwrap_or(0.8).clamp(0.2, 2.0),
                "fov" => s.fov = number.unwrap_or(75.0).clamp(60.0, 100.0),
                "net_stats" => s.net_stats = flag,
                "pixel_size" => s.pixel_size = number.unwrap_or(3.0).clamp(1.0, 6.0) as u32,
                "dither" => s.dither = flag,
                "shadows" => s.shadows = flag,
                "fullscreen" => s.fullscreen = flag,
                "vsync" => s.vsync = flag,
                "volume" => s.volume = number.unwrap_or(0.8).clamp(0.0, 1.0),
                "effects" => s.full_effects = value != "reduced",
                "aim_assist" => s.aim_assist = value != "off",
                "stick_curve" => s.dynamic_curve = value != "standard",
                "callsign" => s.callsign = sanitize(value, 16),
                "last_address" => s.last_address = sanitize(value, 64),
                "token" => s.token = value.parse().unwrap_or(0),
                "class" => s.classes.push(sanitize(value, 200)),
                _ => s.bindings.load_line(key, value),
            }
        }
        s.ensure_token();
        s
    }

    /// Makes up a reconnect token the first time the game runs, and saves it.
    fn ensure_token(&mut self) {
        if self.token != 0 {
            return;
        }
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        let mut x = time ^ ((std::process::id() as u64) << 32) ^ (self as *const Self as u64);
        // splitmix64 finaliser
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        self.token = (x ^ (x >> 31)).max(1);
        self.save();
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        let text = format!(
            "sensitivity={}\nads_sensitivity={}\nfov={}\nnet_stats={}\ninvert_y={}\npixel_size={}\ndither={}\nshadows={}\nfullscreen={}\nvsync={}\nvolume={}\ncallsign={}\nlast_address={}\n",
            self.sensitivity,
            self.ads_sensitivity,
            self.fov,
            self.net_stats,
            self.invert_y,
            self.pixel_size,
            self.dither,
            self.shadows,
            self.fullscreen,
            self.vsync,
            self.volume,
            self.callsign,
            self.last_address
        );
        let text = text + &format!("effects={}\n", if self.full_effects { "full" } else { "reduced" });
        let text = text + &format!("aim_assist={}\nstick_curve={}\n", if self.aim_assist { "on" } else { "off" }, if self.dynamic_curve { "dynamic" } else { "standard" });
        let text = text + &format!("token={}\n", self.token) + &self.bindings.save_lines();
        let text = text + &self.classes.iter().map(|c| format!("class={c}\n")).collect::<String>();
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(path, text);
    }
}

pub fn sanitize(text: &str, max: usize) -> String {
    text.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(max).collect()
}
