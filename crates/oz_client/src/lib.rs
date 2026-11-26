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
//! * [`avatar`] - the TV-headed robot operators and their animation rig.
//! * [`showroom`] - an off-screen stage behind menus, lobby and gunsmith.
//! * [`menu`], [`lobby`], [`loadout`], [`gunsmith`], [`stations`], [`hud`] - the screens.
//! * [`retro`] - the ordered-dither post-process.
//! * [`audio`] - procedural sound playback.

pub mod actors;
pub mod art;
pub mod chat;
pub mod cover;
pub mod avatar;
pub mod bindings;
#[cfg(feature = "audio")]
pub mod audio;
pub mod fx;
pub mod gunsmith;
pub mod hud;
pub mod input;
pub mod juice;
pub mod launch;
pub mod loadout;
pub mod lobby;
pub mod predict;
pub mod reconnect;
pub mod menu;
pub mod retro;
pub mod scene;
pub mod settings;
pub mod showroom;
pub mod spectate;
pub mod stations;
pub mod tags;
pub mod ui;
pub mod view;
pub mod wreck;

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
            bindings::BindingsPlugin,
            reconnect::ReconnectPlugin,
            cover::CoverPlugin,
            art::ArtPlugin,
            view::ViewPlugin,
            scene::ScenePlugin,
            actors::ActorsPlugin,
            fx::FxPlugin,
            ui::UiKitPlugin,
            menu::MenuPlugin,
            stations::StationsPlugin,
            hud::HudPlugin,
            loadout::LoadoutPlugin,
        ))
        .add_plugins((avatar::AvatarPlugin, showroom::ShowroomPlugin, gunsmith::GunsmithPlugin, lobby::LobbyPlugin, spectate::SpectatePlugin, predict::PredictPlugin, chat::ChatPlugin, tags::TagsPlugin, juice::JuicePlugin, wreck::WreckPlugin,
        ));
        #[cfg(feature = "audio")]
        app.add_plugins(audio::AudioFxPlugin);
    }
}

#[derive(Resource, Clone)]
pub struct GameTitle(pub String);
