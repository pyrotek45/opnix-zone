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
use oz_sim::{net::{self, JoinStatus}, prelude::*, session::end_match};

const THREATS: [(&str, f32); 4] = [("RECRUIT", 0.75), ("STANDARD", 1.0), ("VETERAN", 1.35), ("NIGHTMARE", 1.8)];

#[derive(Resource)]
pub struct MenuState {
    pub mode: usize,
    pub biome: usize,
    pub threat: usize,
    pub team_size: u8,
    editing: Option<Field>,
    status: String,
}

impl Default for MenuState {
    fn default() -> Self {
        Self { mode: 0, biome: 0, threat: 1, team_size: 2, editing: None, status: String::new() }
    }
}

/// Label for the system that resets the screen when a match starts.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnterMatch;

/// Label for the menu's input systems; station input runs before them.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MenuActions;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuState>()
            .init_resource::<JoinStatus>()
            .add_systems(OnEnter(MatchPhase::Active), (|mut screen: ResMut<Screen>| *screen = Screen::None).in_set(EnterMatch))
            .add_systems(OnExit(MatchPhase::Active), |mut screen: ResMut<Screen>| *screen = Screen::Main)
            .add_systems(Update, (type_text, toggle_pause, menu_actions, watch_match, apply_settings).chain().in_set(MenuActions).in_set(ClientSet::Input).after(crate::input::ReadDevices))
            .add_systems(
                Update,
                (ui::clear_screen, build_menu, crate::stations::build_station, crate::loadout::build_loadout, ui::finish_rebuild).chain().in_set(ClientSet::Present),
            );
    }
}

fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64)
}

fn make_rules(modes: &Modes, menu: &MenuState) -> Option<Rules> {
    let mode = modes.0.get(menu.mode).filter(|mode| mode.playable)?;
    let mut rules = (mode.rules)(seed());
    rules.mode = menu.mode as u8;
    rules.recipe.biome = Biome::ALL[menu.biome % Biome::ALL.len()];
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
            Field::Callsign => (&mut settings.callsign, 16),
            Field::Address => (&mut settings.last_address, 64),
        };
        match &event.logical_key {
            Key::Character(typed) => *text = sanitize(&format!("{text}{typed}"), max),
            Key::Space if field == Field::Callsign => *text = sanitize(&format!("{text} "), max),
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

/// Shows the game-over screen, and pauses time for single-player menus.
fn watch_match(
    state: Option<Res<MatchState>>,
    role: Res<NetRole>,
    phase: Res<State<MatchPhase>>,
    mut screen: ResMut<Screen>,
    mut time: ResMut<Time<Virtual>>,
    join: Res<JoinStatus>,
    mut dirty: ResMut<UiDirty>,
    mut shown: Local<bool>,
) {
    let active = *phase.get() == MatchPhase::Active;
    let lost = active && state.is_some_and(|state| state.over());
    if lost && !*shown && *screen == Screen::None {
        *screen = Screen::GameOver;
    }
    *shown = lost;
    let pause = active && *role == NetRole::Offline && matches!(*screen, Screen::Pause | Screen::Settings);
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
) {
    let in_match = *phase.get() == MatchPhase::Active;
    for ActEvent(act) in events.read() {
        let cycle = |value: usize, step: i32, len: usize| (value as i32 + step).rem_euclid(len as i32) as usize;
        match act {
            Act::Go(target) => {
                *screen = target.clone();
                menu.status.clear();
            }
            Act::Back => match *screen {
                Screen::Join => *screen = Screen::Main,
                Screen::Settings => *screen = if in_match { Screen::Pause } else { Screen::Main },
                Screen::Pause => *screen = Screen::None,
                Screen::GameOver => *screen = Screen::None,
                _ => {}
            },
            Act::Mode(step) => menu.mode = cycle(menu.mode, *step, modes.0.len()),
            Act::Biome(step) => menu.biome = cycle(menu.biome, *step, Biome::ALL.len()),
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
            Act::Connect => {
                settings.save();
                net::join(&mut commands, &normalize_address(&settings.last_address), settings.callsign.clone());
            }
            Act::Edit(field) => menu.editing = if menu.editing == Some(*field) { None } else { Some(*field) },
            Act::Set(setting, step) => {
                let step = *step as f32;
                match setting {
                    Setting::Sensitivity => settings.sensitivity = (settings.sensitivity + 0.1 * step).clamp(0.2, 3.0),
                    Setting::InvertY => settings.invert_y = !settings.invert_y,
                    Setting::PixelSize => settings.pixel_size = (settings.pixel_size as i32 + step as i32 - 1).rem_euclid(6) as u32 + 1,
                    Setting::Dither => settings.dither = !settings.dither,
                    Setting::Shadows => settings.shadows = !settings.shadows,
                    Setting::Fullscreen => settings.fullscreen = !settings.fullscreen,
                    Setting::Vsync => settings.vsync = !settings.vsync,
                    Setting::Volume => settings.volume = (settings.volume + 0.1 * step).clamp(0.0, 1.0),
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
            ui::root(&mut commands, 1.0).with_children(|root| {
                ui::label(root, title.0.clone(), 44.0, ui::TEXT);
                ui::label(root, "A MODULAR, FULLY PROCEDURAL SHOOTER", 15.0, ui::ACCENT);
                root.spawn(ui::panel(560.0)).with_children(|panel| {
                    ui::button(panel, order, "PLAY SOLO", Act::Solo);
                    ui::button(panel, order, format!("HOST GAME   (PORT {DEFAULT_PORT})"), Act::Host);
                    ui::button(panel, order, "JOIN GAME", Act::Go(Screen::Join));
                    ui::stepper(panel, order, format!("MODE     <  {}  >", mode.name), Act::Mode(-1), Act::Mode(1));
                    ui::label(panel, mode.blurb, 14.0, if mode.playable { ui::DIM } else { ui::WARN });
                    ui::stepper(panel, order, format!("MAP      <  {}  >", Biome::ALL[menu.biome].look().title), Act::Biome(-1), Act::Biome(1));
                    ui::label(panel, Biome::ALL[menu.biome].look().blurb, 14.0, ui::DIM);
                    let versus = mode.playable && (mode.rules)(0).pvp;
                    if versus {
                        let n = menu.team_size;
                        ui::stepper(panel, order, format!("TEAMS    <  {n} V {n}  >"), Act::TeamSize(-1), Act::TeamSize(1));
                        ui::label(panel, "BOTS FILL EVERY SLOT A HUMAN DOES NOT TAKE", 14.0, ui::DIM);
                    }
                    ui::stepper(panel, order, format!("{}   <  {}  >", if versus { "BOTS  " } else { "THREAT" }, THREATS[menu.threat].0), Act::Difficulty(-1), Act::Difficulty(1));
                    ui::button(panel, order, "SETTINGS", Act::Go(Screen::Settings));
                    ui::button(panel, order, "QUIT", Act::Quit);
                    if !menu.status.is_empty() {
                        ui::label(panel, menu.status.clone(), 14.0, ui::WARN);
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
                JoinStatus::Failed(why) => (why.clone(), ui::WARN),
            };
            ui::root(&mut commands, 1.0).with_children(|root| {
                ui::label(root, "JOIN GAME", 34.0, ui::TEXT);
                root.spawn(ui::panel(560.0)).with_children(|panel| {
                    ui::button(panel, order, field("CALLSIGN", &settings.callsign, menu.editing == Some(Field::Callsign)), Act::Edit(Field::Callsign));
                    ui::button(panel, order, field("ADDRESS", &settings.last_address, menu.editing == Some(Field::Address)), Act::Edit(Field::Address));
                    ui::label(panel, format!("HOST IP OR NAME. PORT {DEFAULT_PORT} IS USED UNLESS YOU ADD :PORT"), 13.0, ui::DIM);
                    ui::button(panel, order, "CONNECT", Act::Connect);
                    ui::button(panel, order, "BACK", Act::Back);
                    ui::label(panel, status.0, 14.0, status.1);
                });
            });
        }
        Screen::Settings => {
            let set = |setting| (Act::Set(setting, -1), Act::Set(setting, 1));
            let dim = if *phase.get() == MatchPhase::Active { 0.75 } else { 1.0 };
            ui::root(&mut commands, dim).with_children(|root| {
                ui::label(root, "SETTINGS", 34.0, ui::TEXT);
                root.spawn(ui::panel(560.0)).with_children(|panel| {
                    let mut value = |text: String, setting: Setting| {
                        let (left, right) = set(setting);
                        ui::stepper(panel, order, text, left, right);
                    };
                    value(format!("LOOK SENSITIVITY   <  {:.1}  >", settings.sensitivity), Setting::Sensitivity);
                    value(format!("INVERT Y           <  {}  >", on_off(settings.invert_y)), Setting::InvertY);
                    value(format!("PIXEL SIZE         <  {}  >", settings.pixel_size), Setting::PixelSize);
                    value(format!("RETRO DITHER       <  {}  >", on_off(settings.dither)), Setting::Dither);
                    value(format!("SHADOWS            <  {}  >", on_off(settings.shadows)), Setting::Shadows);
                    value(format!("FULLSCREEN         <  {}  >", on_off(settings.fullscreen)), Setting::Fullscreen);
                    value(format!("VSYNC              <  {}  >", on_off(settings.vsync)), Setting::Vsync);
                    value(format!("VOLUME             <  {:.0}%  >", settings.volume * 100.0), Setting::Volume);
                    ui::label(panel, "A BIGGER PIXEL SIZE RENDERS FEWER PIXELS: THE FASTEST WAY TO GAIN FRAME RATE.", 13.0, ui::DIM);
                    ui::button(panel, order, "BACK", Act::Back);
                });
            });
        }
        Screen::Pause => {
            let net = match &*link {
                NetLink::Host(host) => format!("HOSTING ON PORT {DEFAULT_PORT} - {} GUEST(S) CONNECTED", host.peers.len()),
                NetLink::Client(_) => "CONNECTED TO A HOST - THE MATCH KEEPS RUNNING".into(),
                NetLink::None => "SOLO - TIME IS FROZEN".into(),
            };
            ui::root(&mut commands, 0.6).with_children(|root| {
                ui::label(root, "PAUSED", 34.0, ui::TEXT);
                root.spawn(ui::panel(420.0)).with_children(|panel| {
                    ui::button(panel, order, "RESUME", Act::Go(Screen::None));
                    ui::button(panel, order, "SETTINGS", Act::Go(Screen::Settings));
                    ui::button(panel, order, "LEAVE MATCH", Act::Leave);
                    ui::label(panel, net, 13.0, ui::DIM);
                });
            });
        }
        Screen::GameOver => {
            let (headline, tint, summary) = match state.as_deref() {
                Some(state) if state.phase == Phase::Won => (
                    format!("TEAM {} WINS", if state.winner == 0 { "TEAL" } else { "CORAL" }),
                    ui::GOOD,
                    format!("TEAL {}   |   CORAL {}", state.scores[0], state.scores[1]),
                ),
                Some(state) => ("THE LINE HAS FALLEN".to_string(), ui::WARN, format!("REACHED NIGHT {}   |   {} KILLS", state.day, state.kills)),
                None => (String::new(), ui::WARN, String::new()),
            };
            ui::root(&mut commands, 0.7).with_children(|root| {
                ui::label(root, headline, 40.0, tint);
                ui::label(root, summary, 18.0, ui::TEXT);
                root.spawn(ui::panel(420.0)).with_children(|panel| {
                    ui::button(panel, order, "RETURN TO MENU", Act::Leave);
                    ui::button(panel, order, "KEEP WATCHING", Act::Go(Screen::None));
                });
            });
        }
        Screen::None | Screen::Station { .. } | Screen::Loadout => {}
    }
}
