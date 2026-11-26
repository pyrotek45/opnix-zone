//! `oz_client` - everything a player sees, hears and touches.
//!
//! The client never changes gameplay state directly. It reads the simulation
//! (`oz_sim`), draws it, and turns keyboard / mouse / gamepad input into
//! `PlayerInput` and `PlayerCommand`s. Because host and client simulate the
//! same components, this crate does not care which side of the network it is on.
//!
//! * [`settings`] - persistent options.
//! * [`input`] - one `Controls` resource fed by keyboard, mouse and gamepad.
//! * [`art`] - bakes block models into batched, vertex-coloured meshes.
//! * [`scene`] - terrain, water, props, sky and the day/night lighting.
//! * [`actors`] - visuals for enemies, structures, ore, projectiles, players.
//! * [`view`] - first-person camera, low-resolution render target, viewmodel.
//! * [`fx`] - tracers, particles and explosions.
//! * [`ui`] - a small widget kit with mouse, keyboard and gamepad focus.
//! * [`menu`], [`stations`], [`hud`] - the screens built from that kit.
//! * [`retro`] - the ordered-dither post-process.
//! * [`audio`] - procedural sound playback.

pub mod actors;
pub mod art;
#[cfg(feature = "audio")]
pub mod audio;
pub mod fx;
pub mod hud;
pub mod input;
pub mod menu;
pub mod retro;
pub mod scene;
pub mod settings;
pub mod stations;
pub mod ui;
pub mod view;

use bevy::{prelude::*, window::PresentMode};

/// Client-side frame ordering, all of it after the simulation has run.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClientSet {
    /// Read devices, drive menus, write `PlayerInput`.
    Input,
    /// Spawn and animate visuals from simulation state.
    Present,
}

pub struct ClientPlugin {
    pub title: String,
}

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        let settings = settings::Settings::load();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: self.title.clone(),
                        resolution: (1280.0_f32, 720.0_f32).into(),
                        present_mode: if settings.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync },
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .insert_resource(settings)
        .insert_resource(GameTitle(self.title.clone()))
        .configure_sets(Update, ClientSet::Input.before(oz_sim::SimSet::Receive))
        .configure_sets(Update, ClientSet::Present.after(oz_sim::SimSet::Send))
        .add_plugins((
            retro::RetroPlugin,
            input::InputPlugin,
            art::ArtPlugin,
            view::ViewPlugin,
            scene::ScenePlugin,
            actors::ActorsPlugin,
            fx::FxPlugin,
            ui::UiKitPlugin,
            menu::MenuPlugin,
            stations::StationsPlugin,
            hud::HudPlugin,
        ));
        #[cfg(feature = "audio")]
        app.add_plugins(audio::AudioFxPlugin);
    }
}

#[derive(Resource, Clone)]
pub struct GameTitle(pub String);
