//! Front end: main menu, join screen, settings, pause and game over.

use crate::{
    ClientSet, GameTitle,
    input::Controls,
    settings::{Settings, sanitize},
    stations::Placement,
    ui::{self, Act, ActEvent, Field, Order, Screen, Setting, UiDirty},
};
use bevy::{
    app::AppExit,
    input::keyboard::{Key, KeyboardInput},
    prelude::*,
    window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode},
};
use oz_net::{DEFAULT_PORT, normalize_address};
use oz_sim::{
    net::{self, JoinStatus},
    prelude::*,
    session::end_match,
};

const THREATS: [(&str, f32); 4] = [("RECRUIT", 0.75), ("STANDARD", 1.0), ("VETERAN", 1.35), ("NIGHTMARE", 1.8)];

#[derive(Resource)]
pub struct MenuState {
    pub mode: usize,
    pub biome: usize,
    /// 0 = the map's own weather, then each `Weather` in turn.
    pub weather: usize,
    /// Versus: index into `Layout::VERSUS` (random town or a fixed map).
    pub layout: usize,
    pub threat: usize,
    pub team_size: u8,
    editing: Option<Field>,
    status: String,
    /// Games answering on the local network, and the search in progress.
    found: Vec<oz_net::lan::Found>,
    searching: bool,
}

/// The LAN search socket lives outside `MenuState` so the state stays plain data.
#[derive(Resource)]
struct Search(oz_net::lan::Finder);

impl Default for MenuState {
    fn default() -> Self {
        Self { mode: 0, biome: 0, weather: 0, layout: 0, threat: 1, team_size: 2, editing: None, status: String::new(), found: Vec::new(), searching: false }
    }
}

/// Label for the system that resets the screen when a match starts.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnterMatch;

/// Label for the menu's input systems; station input runs before them.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MenuActions;

/// Label for the systems that rebuild screens.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct BuildScreens;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuState>()
            .init_resource::<JoinStatus>()
            .add_systems(OnEnter(MatchPhase::Active), enter_match.in_set(EnterMatch))
            .add_systems(OnExit(MatchPhase::Active), |mut screen: ResMut<Screen>, pending: Res<PendingMatch>| {
                // A lobby reloading on new rules stays on screen; leaving goes back to the menu.
                if !pending.0.as_ref().is_some_and(|setup| setup.carry.is_some()) {
                    *screen = Screen::Main;
                }
            })
            .add_systems(Update, (type_text, toggle_pause, menu_actions, watch_match, apply_settings, search_lan).chain().in_set(MenuActions).in_set(ClientSet::Input).after(crate::input::ReadDevices))
            .add_systems(
                Update,
                (
                    choose_stage,
                    ui::clear_screen,
                    build_menu,
                    crate::stations::build_station,
                    crate::loadout::build_loadout,
                    crate::gunsmith::build_gunsmith,
                    crate::lobby::build_lobby,
                    crate::naming::build_name,
                    crate::bindings::build_controls,
                    ui::finish_rebuild,
                )
                    .chain()
                    .in_set(BuildScreens)
                    .in_set(ClientSet::Present),
            );
    }
}

/// A new match drops you into it; a relaunched lobby keeps you in the lobby, or wherever
/// you were in its menus (the armory, your name, settings).
fn enter_match(mut screen: ResMut<Screen>, relaunched: Res<Relaunched>, mut dirty: ResMut<UiDirty>, focused: Res<ui::Focused>, mut request: ResMut<ui::FocusRequest>) {
    if !relaunched.0 {
        *screen = Screen::None;
    } else if !matches!(*screen, Screen::Loadout | Screen::Gunsmith { .. } | Screen::Name | Screen::Settings | Screen::Controls) {
        *screen = Screen::Lobby;
        // The rows may shift (a hunt has no SWITCH TEAM); keep the cursor on the same one.
        request.0 = focused.0.clone();
    }
    dirty.0 = true;
}

fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64)
}

fn make_rules(modes: &Modes, menu: &MenuState) -> Option<Rules> {
    let mode = modes.0.get(menu.mode).filter(|mode| mode.playable)?;
    let mut rules = (mode.rules)(seed());
    rules.mode = menu.mode as u8;
    rules.recipe.biome = Biome::ALL[menu.biome % Biome::ALL.len()];
    let layout = oz_core::worldgen::Layout::VERSUS[menu.layout % oz_core::worldgen::Layout::VERSUS.len()];
    if rules.recipe.layout.is_town() && layout != oz_core::worldgen::Layout::Town {
        rules.recipe = oz_core::worldgen::WorldRecipe::preset(layout, rules.recipe.biome, rules.recipe.weather);
    }
    if rules.horde {
        // Horde maps keep their own look; the layout stepper picks the map, the squad its size.
        rules.recipe = oz_core::worldgen::WorldRecipe::horde(oz_core::worldgen::Layout::HORDE[menu.layout % oz_core::worldgen::Layout::HORDE.len()]);
        rules.team_size = menu.team_size;
        rules.difficulty *= THREATS[menu.threat % THREATS.len()].1;
        return Some(rules);
    }
    if menu.weather > 0 {
        rules.recipe.weather = oz_core::worldgen::Weather::ALL[(menu.weather - 1) % oz_core::worldgen::Weather::ALL.len()];
    }
    if rules.pvp {
        rules.team_size = menu.team_size;
    }
    rules.difficulty *= THREATS[menu.threat % THREATS.len()].1;
    Some(rules)
}

/// Typed characters go to whichever text field is being edited.
fn type_text(mut keys: EventReader<KeyboardInput>, mut menu: ResMut<MenuState>, mut settings: ResMut<Settings>, mut dirty: ResMut<UiDirty>, screen: Res<Screen>) {
    let Some(field) = menu.editing.filter(|_| *screen == Screen::Join) else {
        keys.clear();
        menu.bypass_change_detection().editing = None;
        return;
    };
    for event in keys.read().filter(|event| event.state.is_pressed()) {
        let (text, max) = match field {
            Field::Address => (&mut settings.last_address, 64),
        };
        match &event.logical_key {
            Key::Character(typed) => *text = sanitize(&format!("{text}{typed}"), max),
            Key::Backspace => {
                text.pop();
            }
            Key::Enter | Key::Tab => menu.editing = None,
            _ => continue,
        }
        dirty.0 = true;
    }
}

fn toggle_pause(controls: Res<Controls>, phase: Res<State<MatchPhase>>, placing: Res<Placement>, mut screen: ResMut<Screen>) {
    if controls.pause && *phase.get() == MatchPhase::Active && *screen == Screen::None && placing.kind.is_none() && !placing.just_closed {
        *screen = Screen::Pause;
    }
}

/// Moves between lobby, match and game-over screens, and pauses time for single-player menus.
#[allow(clippy::too_many_arguments)]
fn watch_match(
    state: Option<Res<MatchState>>,
    role: Res<NetRole>,
    phase: Res<State<MatchPhase>>,
    mut screen: ResMut<Screen>,
    mut time: ResMut<Time<Virtual>>,
    join: Res<JoinStatus>,
    mut dirty: ResMut<UiDirty>,
    mut shown: Local<bool>,
    mut last_phase: Local<Option<Phase>>,
) {
    let active = *phase.get() == MatchPhase::Active;
    let lost = active && state.as_ref().is_some_and(|state| state.over());
    if lost && !*shown && *screen == Screen::None {
        *screen = Screen::GameOver;
    }
    *shown = lost;
    let now = state.as_ref().filter(|_| active).map(|state| state.phase);
    match now {
        // Versus: the lobby is home between matches.
        Some(Phase::Lobby) if matches!(*screen, Screen::None | Screen::GameOver) => *screen = Screen::Lobby,
        // The match started: everyone drops in, wherever they were in the menus.
        Some(Phase::Live) if *last_phase == Some(Phase::Lobby) && *screen != Screen::Pause => *screen = Screen::None,
        _ => {}
    }
    *last_phase = now;
    let pause = active && *role == NetRole::Offline && matches!(*screen, Screen::Pause | Screen::Settings | Screen::Controls | Screen::Name);
    if pause != time.is_paused() {
        if pause { time.pause() } else { time.unpause() }
    }
    if join.is_changed() {
        dirty.0 = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn menu_actions(
    mut events: EventReader<ActEvent>,
    mut screen: ResMut<Screen>,
    mut menu: ResMut<MenuState>,
    mut settings: ResMut<Settings>,
    mut dirty: ResMut<UiDirty>,
    modes: Res<Modes>,
    phase: Res<State<MatchPhase>>,
    mut pending: ResMut<PendingMatch>,
    mut next: ResMut<NextState<MatchPhase>>,
    mut commands: Commands,
    mut exit: EventWriter<AppExit>,
    mut spectate: ResMut<crate::spectate::Spectate>,
) {
    let in_match = *phase.get() == MatchPhase::Active;
    for ActEvent(act) in events.read() {
        let cycle = |value: usize, step: i32, len: usize| (value as i32 + step).rem_euclid(len as i32) as usize;
        match act {
            Act::Go(target) => {
                if *target == Screen::Join {
                    start_search(&mut commands, &mut menu);
                }
                *screen = target.clone();
                menu.status.clear();
            }
            Act::Back => match *screen {
                Screen::Join | Screen::Lineup => *screen = Screen::Main,
                Screen::Settings => *screen = if in_match { Screen::Pause } else { Screen::Main },
                Screen::Controls => *screen = Screen::Settings,
                Screen::Pause | Screen::Chat => *screen = Screen::None,
                Screen::GameOver => *screen = Screen::None,
                _ => {}
            },
            Act::Mode(step) => menu.mode = cycle(menu.mode, *step, modes.0.len()),
            Act::Biome(step) => menu.biome = cycle(menu.biome, *step, Biome::ALL.len()),
            Act::Weather(step) => menu.weather = cycle(menu.weather, *step, oz_core::worldgen::Weather::ALL.len() + 1),
            Act::Layout(step) => menu.layout = cycle(menu.layout, *step, oz_core::worldgen::Layout::VERSUS.len()),
            Act::Difficulty(step) => menu.threat = cycle(menu.threat, *step, THREATS.len()),
            Act::TeamSize(step) => menu.team_size = cycle(menu.team_size as usize - 1, *step, 4) as u8 + 1,
            Act::Solo | Act::Host => {
                let Some(rules) = make_rules(&modes, &menu) else {
                    menu.status = "THAT MODE IS PLANNED BUT NOT PLAYABLE YET".into();
                    dirty.0 = true;
                    continue;
                };
                if *act == Act::Solo {
                    pending.0 = Some(MatchSetup::offline(rules, settings.callsign.clone()));
                } else {
                    match net::host_setup(rules, settings.callsign.clone(), &format!("0.0.0.0:{DEFAULT_PORT}"), false) {
                        Ok(setup) => pending.0 = Some(setup),
                        Err(error) => menu.status = format!("COULD NOT HOST ON PORT {DEFAULT_PORT}: {error}"),
                    }
                }
            }
            Act::FindGames => start_search(&mut commands, &mut menu),
            Act::JoinFound(index) => {
                if let Some(game) = menu.found.get(*index).cloned() {
                    settings.last_address = game.address.to_string();
                    settings.save();
                    net::join_as(&mut commands, &game.address.to_string(), settings.callsign.clone(), settings.token);
                }
            }
            Act::ToggleSpectate => {
                spectate.enabled = !spectate.enabled;
                *screen = Screen::None;
            }
            Act::Connect => {
                settings.save();
                if oz_net::rendezvous::normalize_code(&settings.last_address).is_some() {
                    net::join_code(&mut commands, &settings.last_address, settings.callsign.clone(), settings.token);
                } else {
                    net::join_as(&mut commands, &normalize_address(&settings.last_address), settings.callsign.clone(), settings.token);
                }
            }
            Act::Edit(field) => menu.editing = if menu.editing == Some(*field) { None } else { Some(*field) },
            Act::Set(setting, step) => {
                let step = *step as f32;
                match setting {
                    Setting::Sensitivity => settings.sensitivity = (settings.sensitivity + 0.1 * step).clamp(0.2, 3.0),
                    Setting::AdsSensitivity => settings.ads_sensitivity = (settings.ads_sensitivity + 0.05 * step).clamp(0.2, 2.0),
                    Setting::Fov => settings.fov = (settings.fov + 5.0 * step).clamp(60.0, 100.0),
                    Setting::NetStats => settings.net_stats = !settings.net_stats,
                    Setting::InvertY => settings.invert_y = !settings.invert_y,
                    Setting::PixelSize => settings.pixel_size = (settings.pixel_size as i32 + step as i32 - 1).rem_euclid(6) as u32 + 1,
                    Setting::Dither => settings.dither = !settings.dither,
                    Setting::Shadows => settings.shadows = !settings.shadows,
                    Setting::Fullscreen => settings.fullscreen = !settings.fullscreen,
                    Setting::Vsync => settings.vsync = !settings.vsync,
                    Setting::Volume => settings.volume = (settings.volume + 0.1 * step).clamp(0.0, 1.0),
                    Setting::Effects => settings.full_effects = !settings.full_effects,
                }
                settings.save();
            }
            Act::Leave => end_match(&mut next),
            Act::Quit => {
                exit.write(AppExit::Success);
            }
            _ => continue,
        }
        dirty.0 = true;
    }
}

fn apply_settings(settings: Res<Settings>, mut windows: Query<&mut Window, With<PrimaryWindow>>, mut cameras: Query<&mut crate::retro::Retro>) {
    if !settings.is_changed() {
        return;
    }
    if let Ok(mut window) = windows.single_mut() {
        let mode = if settings.fullscreen { WindowMode::BorderlessFullscreen(MonitorSelection::Current) } else { WindowMode::Windowed };
        if window.mode != mode {
            window.mode = mode;
        }
        let present = if settings.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
        if window.present_mode != present {
            window.present_mode = present;
        }
    }
    for mut retro in &mut cameras {
        retro.enabled = settings.dither as u8 as f32;
    }
}

#[allow(clippy::too_many_arguments)]
fn build_menu(
    mut commands: Commands,
    dirty: Res<UiDirty>,
    screen: Res<Screen>,
    menu: Res<MenuState>,
    settings: Res<Settings>,
    title: Res<GameTitle>,
    modes: Res<Modes>,
    join: Res<JoinStatus>,
    state: Option<Res<MatchState>>,
    link: Res<NetLink>,
    phase: Res<State<MatchPhase>>,
    spectate: Res<crate::spectate::Spectate>,
    reconnect: Res<crate::reconnect::Reconnect>,
    (rules, killcam): (Option<Res<Rules>>, Res<crate::killcam::KillCam>),
) {
    if !dirty.0 {
        return;
    }
    let mut order = Order::default();
    let order = &mut order;
    let on_off = |flag: bool| if flag { "ON" } else { "OFF" };
    match &*screen {
        Screen::Main => {
            let mode = &modes.0[menu.mode % modes.0.len().max(1)];
            ui::root_left(&mut commands, 0.0).with_children(|root| {
                ui::label(root, title.0.clone(), 44.0, ui::TEXT);
                ui::label(root, "A MODULAR, FULLY PROCEDURAL SHOOTER", 15.0, ui::ACCENT);
                root.spawn(ui::panel(560.0)).with_children(|panel| {
                    ui::button(panel, order, format!("NAME     {}", settings.callsign), Act::EditName);
                    ui::button(panel, order, "PLAY SOLO", Act::Solo);
                    ui::button(panel, order, format!("HOST GAME   (PORT {DEFAULT_PORT})"), Act::Host);
                    ui::button(panel, order, "JOIN GAME", Act::Go(Screen::Join));
                    ui::stepper(panel, order, format!("MODE     <  {}  >", mode.name), Act::Mode(-1), Act::Mode(1));
                    ui::label(panel, mode.blurb, 14.0, if mode.playable { ui::DIM } else { ui::WARN });
                    ui::stepper(panel, order, format!("MAP      <  {}  >", Biome::ALL[menu.biome].look().title), Act::Biome(-1), Act::Biome(1));
                    ui::label(panel, Biome::ALL[menu.biome].look().blurb, 14.0, ui::DIM);
                    let weather = if menu.weather == 0 { "RANDOM".to_string() } else { oz_core::worldgen::Weather::ALL[(menu.weather - 1) % 4].name().to_string() };
                    ui::stepper(panel, order, format!("WEATHER  <  {weather}  >"), Act::Weather(-1), Act::Weather(1));
                    let versus = mode.playable && (mode.rules)(0).pvp;
                    if mode.playable && (mode.rules)(0).horde {
                        let layout = oz_core::worldgen::Layout::HORDE[menu.layout % oz_core::worldgen::Layout::HORDE.len()];
                        ui::stepper(panel, order, format!("LAYOUT   <  {}  >", layout.title()), Act::Layout(-1), Act::Layout(1));
                        ui::stepper(panel, order, format!("SQUAD    <  {}  >", menu.team_size), Act::TeamSize(-1), Act::TeamSize(1));
                        ui::label(panel, "BOTS JOIN YOUR SQUAD FOR EVERY SEAT A HUMAN DOES NOT TAKE", 14.0, ui::DIM);
                    }
                    if versus {
                        let layout = oz_core::worldgen::Layout::VERSUS[menu.layout % oz_core::worldgen::Layout::VERSUS.len()];
                        ui::stepper(panel, order, format!("LAYOUT   <  {}  >", layout.title()), Act::Layout(-1), Act::Layout(1));
                        let n = menu.team_size;
                        if (mode.rules)(0).hunt {
                            ui::stepper(panel, order, format!("SQUAD    <  {n}  >"), Act::TeamSize(-1), Act::TeamSize(1));
                            ui::label(panel, "BOTS JOIN YOUR SQUAD FOR EVERY SEAT A HUMAN DOES NOT TAKE", 14.0, ui::DIM);
                        } else {
                            ui::stepper(panel, order, format!("TEAMS    <  {n} V {n}  >"), Act::TeamSize(-1), Act::TeamSize(1));
                            ui::label(panel, "BOTS FILL EVERY SLOT A HUMAN DOES NOT TAKE", 14.0, ui::DIM);
                        }
                    }
                    ui::stepper(panel, order, format!("{}   <  {}  >", if versus { "BOTS  " } else { "THREAT" }, THREATS[menu.threat].0), Act::Difficulty(-1), Act::Difficulty(1));
                    ui::button(panel, order, "SETTINGS", Act::Go(Screen::Settings));
                    ui::button(panel, order, "MODEL VIEWER", Act::Go(Screen::Lineup));
                    ui::button(panel, order, "QUIT", Act::Quit);
                    if !menu.status.is_empty() {
                        ui::label(panel, menu.status.clone(), 14.0, ui::WARN);
                    } else if let Some(status) = reconnect.status() {
                        ui::label(panel, status, 14.0, ui::ACCENT);
                    } else if let JoinStatus::Failed(why) = &*join {
                        ui::label(panel, format!("DISCONNECTED: {why}"), 14.0, ui::WARN);
                    }
                    if reconnect.gave_up() {
                        ui::button(panel, order, "RECONNECT", Act::Reconnect);
                    }
                });
                ui::label(root, "MOUSE  |  ARROWS + ENTER  |  D-PAD + A", 13.0, ui::DIM);
            });
        }
        Screen::Join => {
            let field = |name: &str, value: &str, editing: bool| format!("{name:<10}{value}{}", if editing { "_" } else { "" });
            let status = match &*join {
                JoinStatus::Idle => ("CLICK A FIELD TO EDIT IT, THEN CONNECT".to_string(), ui::DIM),
                JoinStatus::Connecting => ("CONNECTING...".to_string(), ui::ACCENT),
                JoinStatus::Finding(what) => (what.clone(), ui::ACCENT),
                JoinStatus::Failed(why) => (why.clone(), ui::WARN),
            };
            ui::root_left(&mut commands, 0.0).with_children(|root| {
                ui::label(root, "JOIN GAME", 34.0, ui::TEXT);
                root.spawn(ui::panel(560.0)).with_children(|panel| {
                    ui::button(panel, order, field("NAME", &settings.callsign, false), Act::EditName);
                    ui::button(panel, order, field("CODE / IP", &settings.last_address, menu.editing == Some(Field::Address)), Act::Edit(Field::Address));
                    ui::label(panel, "THE JOIN CODE FROM THE HOST'S LOBBY (LIKE K7Q-4MZ), OR AN IP ADDRESS", 13.0, ui::DIM);
                    ui::button(panel, order, "CONNECT", Act::Connect);
                    ui::label(panel, status.0, 14.0, status.1);
                    ui::label(panel, "GAMES ON YOUR NETWORK", 15.0, ui::ACCENT);
                    if menu.found.is_empty() {
                        ui::label(panel, if menu.searching { "SEARCHING..." } else { "NONE FOUND. HOSTS ON OTHER NETWORKS: TYPE THEIR JOIN CODE ABOVE." }, 13.0, ui::DIM);
                    }
                    for (index, game) in menu.found.iter().enumerate() {
                        ui::button(panel, order, format!("{:<18} {:<16} {} PLAYER(S)", game.name, game.mode, game.players), Act::JoinFound(index));
                    }
                    ui::button(panel, order, "SEARCH AGAIN", Act::FindGames);
                    ui::button(panel, order, "BACK", Act::Back);
                });
            });
        }
        Screen::Settings => {
            let set = |setting| (Act::Set(setting, -1), Act::Set(setting, 1));
            let in_match = *phase.get() == MatchPhase::Active;
            let mut root = if in_match { ui::root(&mut commands, 0.75) } else { ui::root_left(&mut commands, 0.0) };
            root.with_children(|root| {
                ui::label(root, "SETTINGS", 34.0, ui::TEXT);
                root.spawn(ui::panel(560.0)).with_children(|panel| {
                    ui::button(panel, order, format!("NAME               {}", settings.callsign), Act::EditName);
                    let mut value = |text: String, setting: Setting| {
                        let (left, right) = set(setting);
                        ui::stepper(panel, order, text, left, right);
                    };
                    value(format!("LOOK SENSITIVITY   <  {:.1}  >", settings.sensitivity), Setting::Sensitivity);
                    value(format!("AIM SENSITIVITY    <  {:.2}  >", settings.ads_sensitivity), Setting::AdsSensitivity);
                    value(format!("FIELD OF VIEW      <  {:.0}  >", settings.fov), Setting::Fov);
                    value(format!("FPS / PING         <  {}  >", on_off(settings.net_stats)), Setting::NetStats);
                    value(format!("INVERT Y           <  {}  >", on_off(settings.invert_y)), Setting::InvertY);
                    value(format!("PIXEL SIZE         <  {}  >", settings.pixel_size), Setting::PixelSize);
                    value(format!("RETRO DITHER       <  {}  >", on_off(settings.dither)), Setting::Dither);
                    value(format!("SHADOWS            <  {}  >", on_off(settings.shadows)), Setting::Shadows);
                    value(format!("FULLSCREEN         <  {}  >", on_off(settings.fullscreen)), Setting::Fullscreen);
                    value(format!("VSYNC              <  {}  >", on_off(settings.vsync)), Setting::Vsync);
                    value(format!("VOLUME             <  {:.0}%  >", settings.volume * 100.0), Setting::Volume);
                    value(format!("EFFECTS            <  {}  >", if settings.full_effects { "FULL" } else { "REDUCED" }), Setting::Effects);
                    ui::label(panel, "A BIGGER PIXEL SIZE RENDERS FEWER PIXELS: THE FASTEST WAY TO GAIN FRAME RATE.", 13.0, ui::DIM);
                    ui::button(panel, order, "CONTROLS", Act::Go(Screen::Controls));
                    ui::button(panel, order, "BACK", Act::Back);
                });
            });
        }
        Screen::Pause => {
            let net = match &*link {
                NetLink::Host(host) => format!("{} GUEST(S) CONNECTED\n{}", host.peers.len(), crate::lobby::hosting_info(host)),
                NetLink::Client(_) => "CONNECTED TO A HOST - THE MATCH KEEPS RUNNING".into(),
                NetLink::None => "SOLO - TIME IS FROZEN".into(),
            };
            let versus = state.as_deref().is_some_and(|s| matches!(s.phase, Phase::Live | Phase::Lobby | Phase::Won));
            ui::root(&mut commands, 0.6).with_children(|root| {
                ui::label(root, "PAUSED", 34.0, ui::TEXT);
                root.spawn(ui::panel(420.0)).with_children(|panel| {
                    ui::button(panel, order, "RESUME", Act::Go(Screen::None));
                    if versus {
                        ui::button(panel, order, "CHANGE CLASS", Act::Go(Screen::Loadout));
                    }
                    ui::button(panel, order, if spectate.enabled { "STOP SPECTATING" } else { "SPECTATE" }, Act::ToggleSpectate);
                    ui::button(panel, order, "SETTINGS", Act::Go(Screen::Settings));
                    ui::button(panel, order, "LEAVE MATCH", Act::Leave);
                    ui::label(panel, net, 13.0, ui::DIM);
                });
            });
        }
        Screen::GameOver => {
            let hunt = rules.as_deref().is_some_and(|r| r.hunt);
            let (headline, tint, summary) = match state.as_deref() {
                Some(state) if state.phase == Phase::Won && hunt => {
                    let kills: u16 = state.roster.iter().filter(|r| r.team == 0).map(|r| r.kills).sum();
                    if state.winner == 0 {
                        ("MAP CLEARED".to_string(), ui::GOOD, format!("{kills} HOSTILES DOWN   |   {} LIVES TO SPARE", state.lives))
                    } else {
                        ("MISSION FAILED".to_string(), ui::WARN, format!("{kills} HOSTILES DOWN   |   {} STILL STANDING", state.hostiles))
                    }
                }
                Some(state) if state.phase == Phase::Won && state.winner < 2 && !state.roster.is_empty() => {
                    (format!("TEAM {} WINS", if state.winner == 0 { "TEAL" } else { "CORAL" }), ui::GOOD, format!("TEAL {}   |   CORAL {}", state.scores[0], state.scores[1]))
                }
                Some(state) if rules.as_deref().is_some_and(|r| r.horde) => {
                    let (headline, summary) = crate::zombies_hud::game_over(state);
                    (headline, ui::WARN, summary)
                }
                // Time ran out on level scores.
                Some(state) if state.phase == Phase::Won && !state.roster.is_empty() => ("DRAW".to_string(), ui::TEXT, format!("TEAL {}   |   CORAL {}", state.scores[0], state.scores[1])),
                Some(state) => ("THE LINE HAS FALLEN".to_string(), ui::WARN, format!("REACHED NIGHT {}   |   {} KILLS", state.day, state.kills)),
                None => (String::new(), ui::WARN, String::new()),
            };
            let versus = state.as_deref().is_some_and(|s| s.phase == Phase::Won);
            if versus && !hunt {
                // The play of the match runs behind this (see `killcam`): results on the left.
                let state = state.as_deref().expect("versus");
                let name = |id: u8| state.roster.iter().find(|r| r.id == id).map_or("?".to_string(), |r| r.name.clone());
                ui::root_left(&mut commands, 0.0).with_children(|root| {
                    root.spawn((
                        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), padding: UiRect::all(Val::Px(16.0)), width: Val::Px(470.0), ..default() },
                        BackgroundColor(Color::srgba(0.02, 0.04, 0.06, 0.85)),
                    ))
                    .with_children(|panel| {
                        ui::label(panel, headline, 38.0, tint);
                        ui::label(panel, summary, 18.0, ui::TEXT);
                        if let Some((id, title)) = killcam.best_play() {
                            ui::label(panel, format!("PLAY OF THE MATCH:  {}  -  {title}", name(id)), 15.0, ui::ACCENT);
                        }
                        ui::label(panel, "AWARDS", 20.0, ui::ACCENT);
                        let awards = killcam.awards();
                        if awards.is_empty() {
                            ui::label(panel, "NO AWARDS THIS TIME", 15.0, ui::DIM);
                        }
                        for (award, id, why) in awards {
                            panel.spawn(Node { flex_direction: FlexDirection::Column, padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)), border: UiRect::left(Val::Px(3.0)), ..default() }).insert(BorderColor(ui::ACCENT)).with_children(|row| {
                                ui::label(row, format!("{award}   {}", name(id)), 17.0, ui::TEXT);
                                ui::label(row, why, 13.0, ui::DIM);
                            });
                        }
                        let seconds = state.timer.max(0.0).ceil();
                        ui::label(panel, format!("BACK TO THE LOBBY IN {seconds}"), 15.0, ui::DIM);
                        ui::button(panel, order, "LEAVE MATCH", Act::Leave);
                    });
                });
            } else {
                ui::root(&mut commands, 0.7).with_children(|root| {
                    ui::label(root, headline, 40.0, tint);
                    ui::label(root, summary, 18.0, ui::TEXT);
                    if versus {
                        let seconds = state.as_deref().map_or(0.0, |s| s.timer.max(0.0)).ceil();
                        ui::label(root, format!("BACK TO THE LOBBY IN {seconds}"), 15.0, ui::DIM);
                    }
                    root.spawn(ui::panel(420.0)).with_children(|panel| {
                        ui::button(panel, order, "LEAVE MATCH", Act::Leave);
                        if !versus {
                            ui::button(panel, order, "KEEP WATCHING", Act::Go(Screen::None));
                        }
                    });
                });
            }
        }
        Screen::Lineup => {
            ui::root(&mut commands, 0.0).with_children(|root| {
                root.spawn(Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(24.0),
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|top| {
                    ui::label(top, "OPERATOR MODELS", 30.0, ui::TEXT);
                    ui::label(top, "IDLE  -  RUN  -  CROUCH  -  PRONE  -  AIM UP  -  RELOAD  -  DOWN", 14.0, ui::ACCENT);
                });
                root.spawn(Node { position_type: PositionType::Absolute, bottom: Val::Px(24.0), ..default() }).with_children(|bottom| {
                    ui::sized_button(bottom, order, "BACK", Act::Back, None, Val::Px(160.0));
                });
            });
        }
        Screen::None | Screen::Station { .. } | Screen::Loadout | Screen::Lobby | Screen::Gunsmith { .. } | Screen::Chat | Screen::Controls | Screen::Name => {}
    }
}

/// Picks what the showroom stage shows behind each screen.
fn choose_stage(screen: Res<Screen>, phase: Res<State<MatchPhase>>, state: Option<Res<MatchState>>, players: Query<(&Player, &Arsenal), With<LocalPlayer>>, mut stage: ResMut<crate::showroom::Stage>) {
    use crate::showroom::Stage;
    let in_match = *phase.get() == MatchPhase::Active;
    let me = players.single().ok();
    let team = me.map_or(0, |(p, _)| p.team);
    let wanted = match &*screen {
        // The gunsmith drives the stage itself (it previews parts).
        Screen::Gunsmith { .. } => return,
        Screen::Main | Screen::Join => Stage::Operator { team: 2, weapon: None },
        Screen::Settings | Screen::Controls if !in_match => Stage::Operator { team: 2, weapon: None },
        Screen::Lineup => Stage::Lineup,
        Screen::Lobby => {
            let count = state.as_ref().map_or(1, |s| s.roster.iter().filter(|r| r.team == team).count().max(1) as u8);
            Stage::Squad { team, count }
        }
        // The armory drives the stage itself (it previews what is under the cursor).
        Screen::Loadout => return,
        _ => Stage::Hidden,
    };
    if *stage != wanted {
        *stage = wanted;
    }
}

fn start_search(commands: &mut Commands, menu: &mut MenuState) {
    menu.found.clear();
    match oz_net::lan::Finder::start() {
        Ok(finder) => {
            commands.insert_resource(Search(finder));
            menu.searching = true;
        }
        Err(_) => menu.searching = false,
    }
}

/// Collects LAN replies while a search is running.
fn search_lan(mut commands: Commands, search: Option<ResMut<Search>>, mut menu: ResMut<MenuState>, mut dirty: ResMut<UiDirty>) {
    let Some(mut search) = search else { return };
    let running = search.0.poll();
    if search.0.found != menu.found {
        menu.found = search.0.found.clone();
        dirty.0 = true;
    }
    if !running {
        commands.remove_resource::<Search>();
        menu.searching = false;
        dirty.0 = true;
    }
}
