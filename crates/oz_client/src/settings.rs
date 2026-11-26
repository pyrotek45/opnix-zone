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
    pub callsign: String,
    pub last_address: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sensitivity: 1.0, ads_sensitivity: 0.8, fov: 75.0, net_stats: false, invert_y: false, pixel_size: 3, dither: true, shadows: true, fullscreen: false, vsync: true, volume: 0.8,
            callsign: "OPERATOR".into(), last_address: "127.0.0.1".into(),
        }
    }
}

fn path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .map(|dir| dir.join("opnix-zone").join("settings.txt"))
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Self::default();
        let Some(text) = path().and_then(|p| fs::read_to_string(p).ok()) else { return s };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
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
                "callsign" => s.callsign = sanitize(value, 16),
                "last_address" => s.last_address = sanitize(value, 64),
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        let text = format!(
            "sensitivity={}\nads_sensitivity={}\nfov={}\nnet_stats={}\ninvert_y={}\npixel_size={}\ndither={}\nshadows={}\nfullscreen={}\nvsync={}\nvolume={}\ncallsign={}\nlast_address={}\n",
            self.sensitivity, self.ads_sensitivity, self.fov, self.net_stats, self.invert_y, self.pixel_size, self.dither, self.shadows, self.fullscreen, self.vsync, self.volume, self.callsign, self.last_address
        );
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(path, text);
    }
}

pub fn sanitize(text: &str, max: usize) -> String {
    text.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(max).collect()
}
