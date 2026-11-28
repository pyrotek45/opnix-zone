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
//! * [`punch`] - heavy combat feedback: hit-stop, kill confirms, damage numbers, big blasts.
//! * [`ui`] - a small widget kit with mouse, keyboard and gamepad focus.
//! * [`avatar`] - the TV-headed robot operators and their animation rig.
//! * [`showroom`] - an off-screen stage behind menus, lobby and gunsmith.
//! * [`objectives`] - domination flags, search and destroy's bomb, rounds and markers.
//! * [`menu`], [`lobby`], [`naming`], [`loadout`], [`gunsmith`], [`stations`], [`hud`] - the screens.
//! * [`retro`] - the ordered-dither post-process.
//! * [`audio`] - procedural sound playback.

pub mod actors;
pub mod ambient;
#[cfg(feature = "audio")]
pub mod announcer;
pub mod art;
#[cfg(feature = "audio")]
pub mod audio;
pub mod avatar;
pub mod bindings;
pub mod bushes;
pub mod career;
pub mod chat;
pub mod clipboard;
pub mod cover;
pub mod debuglog;
pub mod devshots;
pub mod frametime;
pub mod fx;
pub mod gunsmith;
pub mod hud;
pub mod input;
pub mod juice;
pub mod killcam;
pub mod launch;
pub mod loadout;
pub mod lobby;
pub mod menu;
pub mod naming;
pub mod objectives;
pub mod optics;
pub mod predict;
pub mod qol;
pub mod props;
pub mod punch;
pub mod reconnect;
pub mod retro;
pub mod scene;
pub mod settings;
pub mod showroom;
pub mod spectate;
pub mod stations;
pub mod tactical;
pub mod tags;
pub mod typeface;
pub mod ui;
pub mod view;
pub mod wreck;
pub mod zombies_hud;

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
                .set(ImagePlugin::default_nearest())
                .set(bevy::log::LogPlugin { custom_layer: debuglog::log_layer, ..default() }),
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
        .add_plugins((
            avatar::AvatarPlugin,
            showroom::ShowroomPlugin,
            gunsmith::GunsmithPlugin,
            lobby::LobbyPlugin,
            spectate::SpectatePlugin,
            predict::PredictPlugin,
            chat::ChatPlugin,
            tags::TagsPlugin,
            juice::JuicePlugin,
            wreck::WreckPlugin,
            ambient::AmbientPlugin,
            optics::OpticsPlugin,
            props::PropsPlugin,
            tactical::TacticalPlugin,
        ));
        app.add_plugins((devshots::DevShotsPlugin, debuglog::DebugLogPlugin, frametime::FrameTimePlugin));
        app.add_plugins((killcam::KillCamPlugin, bushes::BushesPlugin, career::CareerPlugin, typeface::TypefacePlugin, qol::QolPlugin, naming::NamingPlugin, objectives::ObjectivesPlugin, punch::PunchPlugin));
        #[cfg(feature = "audio")]
        app.add_plugins((audio::AudioFxPlugin, announcer::AnnouncerPlugin));
    }
}

#[derive(Resource, Clone)]
pub struct GameTitle(pub String);
