//! The versus lobby and the scoreboard.
//!
//! The lobby is where every game choice is made (the main menu only hosts, joins
//! and names you). The lobby code sits at the top right, big, a click to copy; under it
//! both teams side by side as roster cards. The list on the left starts with the MODE and
//! everything under it follows from it: the map type (a RANDOM TOWN by size and theme,
//! with NEW MAP to roll another, or a SET MAP by name; co-op modes play their own set
//! maps), the team size and a MATCH SETTINGS page with only the settings that mode uses.
//! Then your own rows: start (host) / ready (guests), class, team, name, leave. Guests see
//! the host's values read-only. A co-op pick (horde) starts its own match from here and
//! everyone comes back here afterwards, as after every match. Everything is a `LobbyAction` command,
//! so it works the same for the host, for guests and offline. Holding Tab
//! (Back on a pad) during a match shows the scoreboard.

use crate::{
    ClientSet,
    input::Controls,
    ui::{self, Act, ActEvent, FocusRequest, Order, Screen, UiDirty},
};
use bevy::{ecs::hierarchy::ChildSpawnerCommands, prelude::*};
use oz_sim::prelude::*;

/// What to tell friends: the LAN address, and the internet address if the router opened the port.
pub fn hosting_info(host: &oz_sim::net::HostLink) -> String {
    let code = host.join_code().map_or(String::new(), |code| format!("JOIN CODE  {}      ", oz_net::rendezvous::pretty_code(&code)));
    format!("{code}{}", host_address(host))
}

/// The host's addresses (the lobby shows its code on its own, big).
pub fn host_address(host: &oz_sim::net::HostLink) -> String {
    use oz_net::lan::{PortStatus, local_ip};
    let lan = local_ip().map_or("NO NETWORK".to_string(), |ip| format!("{ip}:{}", host.port));
    let internet = match (host.router_status(), host.public_addr()) {
        (Some(PortStatus::Open(ip)), _) => format!("DIRECT  {ip}:{}", host.port),
        (_, Some(public)) => format!("DIRECT  {public}"),
        (Some(PortStatus::Trying), None) => "FINDING INTERNET ADDRESS...".to_string(),
        (Some(_), None) => "NO INTERNET ADDRESS YET".to_string(),
        (None, None) => "LOCAL ONLY".to_string(),
    };
    format!("LAN  {lan}    {internet}")
}

pub const TEAM_NAMES: [&str; 2] = ["TEAL", "CORAL"];
pub const TEAM_COLORS: [Color; 2] = [Color::srgb(0.25, 0.8, 0.82), Color::srgb(0.95, 0.52, 0.42)];

#[derive(Component)]
struct Scoreboard;

/// Which page of the lobby is open: 0 the main list, 1 the match settings.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub struct LobbyPage(pub u8);

/// The last click on the lobby code: whether it reached the clipboard, and when.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct Copied(pub Option<(bool, f32)>);

/// How long "COPIED" shows after a click on the lobby code.
const COPIED_FOR: f32 = 2.5;

/// The code friends join this lobby with: the host's own, or the one a guest joined with.
pub fn lobby_code(link: &NetLink) -> Option<String> {
    match link {
        NetLink::Host(host) => host.join_code(),
        NetLink::Client(client) => client.code.clone(),
        NetLink::None => None,
    }
}

/// The host's join code as the lobby shows it: spaced out for reading aloud.
pub fn spaced_code(code: &str) -> String {
    let pretty = oz_net::rendezvous::pretty_code(code);
    let spaced: Vec<String> = pretty.split('-').map(|half| half.chars().map(String::from).collect::<Vec<_>>().join(" ")).collect();
    spaced.join("  -  ")
}

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LobbyPage>()
            .init_resource::<Copied>()
            .add_systems(Update, remember_lobby.run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, lobby_actions.in_set(ClientSet::Input).after(crate::input::ReadDevices).before(crate::menu::MenuActions).run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, (refresh_lobby.before(crate::menu::BuildScreens), scoreboard).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

#[allow(clippy::too_many_arguments)]
fn lobby_actions(
    mut events: EventReader<ActEvent>,
    screen: Res<Screen>,
    role: Res<NetRole>,
    players: Query<&Player, With<LocalPlayer>>,
    mut orders: EventWriter<CommandEvent>,
    mut page: ResMut<LobbyPage>,
    mut request: ResMut<FocusRequest>,
    mut dirty: ResMut<UiDirty>,
    (link, time, mut copied): (Res<NetLink>, Res<Time<Real>>, ResMut<Copied>),
) {
    // The pause menu and the results carry BACK TO LOBBY in a co-op match.
    if !matches!(*screen, Screen::Lobby | Screen::GameOver | Screen::Pause) {
        return;
    }
    let Ok(player) = players.single() else { return };
    for ActEvent(act) in events.read() {
        match act {
            Act::Lobby(action) => {
                orders.write(CommandEvent { player: player.id, command: PlayerCommand::Lobby(*action) });
            }
            Act::LobbyPage(to) if *screen == Screen::Lobby => {
                // Into the settings: the cursor lands on the first one. Back out: on the entry.
                request.0 = Some(match to {
                    1 if *role != NetRole::Client => Act::Lobby(LobbyAction::TeamSize(1)),
                    1 => Act::LobbyPage(0),
                    _ => Act::LobbyPage(1),
                });
                page.0 = *to;
                dirty.0 = true;
            }
            Act::CopyCode => {
                if let Some(code) = lobby_code(&link) {
                    copied.0 =Some((crate::clipboard::copy(&oz_net::rendezvous::pretty_code(&code)), time.elapsed_secs()));
                    dirty.0 = true;
                }
            }
            Act::Back if *screen == Screen::Lobby && page.0 != 0 => {
                request.0 = Some(Act::LobbyPage(1));
                page.0 = 0;
                dirty.0 = true;
            }
            _ => {}
        }
    }
}

/// The host's lobby choices (mode, map) are kept for the next PLAY SOLO or HOST GAME.
fn remember_lobby(rules: Res<Rules>, role: Res<NetRole>, mut settings: ResMut<crate::settings::Settings>) {
    if !rules.is_changed() || *role == NetRole::Client || !rules.pvp {
        return;
    }
    let map = oz_sim::versus::map_choice(&rules);
    if settings.lobby_mode != rules.mode || settings.lobby_map != map {
        settings.lobby_mode = rules.mode;
        settings.lobby_map = map;
        settings.save();
    }
}

/// Rebuild the lobby whenever anything on it changes.
#[allow(clippy::type_complexity)]
fn refresh_lobby(
    screen: Res<Screen>,
    state: Res<MatchState>,
    rules: Res<Rules>,
    link: Res<NetLink>,
    mut dirty: ResMut<UiDirty>,
    mut page: ResMut<LobbyPage>,
    (mut copied, time): (ResMut<Copied>, Res<Time<Real>>),
    mut last: Local<Option<(Vec<RosterEntry>, u8, i32, Phase, String, (u8, u8), (Rules, u8, u8, bool))>>,
) {
    if *screen != Screen::Lobby {
        *last = None;
        if *screen != Screen::Name && page.0 != 0 {
            page.0 = 0;
        }
        return;
    }
    if copied.0.is_some_and(|(_, at)| time.elapsed_secs() - at > COPIED_FOR) {
        copied.0 = None;
        dirty.0 = true;
    }
    let router = match &*link {
        NetLink::Host(host) => format!("{:?}", host.router_status()),
        _ => String::new(),
    };
    // Ping changes constantly; leave it out so the lobby does not rebuild every frame.
    let roster: Vec<RosterEntry> = state.roster.iter().map(|r| RosterEntry { ping: 0, weapon: 0, parts: Default::default(), ..r.clone() }).collect();
    let picking = (rules.clone(), state.next_mode, state.next_map, state.change_in > 0.0);
    let signature = (roster, state.team_size, state.countdown.ceil() as i32, state.phase, router, (state.time_mode, state.weather_mode), picking);
    if last.as_ref() != Some(&signature) {
        if last.is_some() {
            dirty.0 = true;
        }
        *last = Some(signature);
    }
}

fn card(parent: &mut ChildSpawnerCommands, entry: &RosterEntry, me: u8, show_score: bool) {
    let team = (entry.team as usize).min(1);
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(7.0)),
                column_gap: Val::Px(10.0),
                align_items: AlignItems::Center,
                border: UiRect::left(Val::Px(5.0)),
                ..default()
            },
            BackgroundColor(if entry.id == me { Color::srgba(0.2, 0.26, 0.22, 0.95) } else { Color::srgba(0.06, 0.1, 0.13, 0.92) }),
            BorderColor(TEAM_COLORS[team]),
        ))
        .with_children(|row| {
            // A tiny TV-head badge in team colours.
            row.spawn((Node { width: Val::Px(18.0), height: Val::Px(14.0), padding: UiRect::all(Val::Px(3.0)), ..default() }, BackgroundColor(TEAM_COLORS[team]))).with_children(|tv| {
                tv.spawn((Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, BackgroundColor(Color::srgb(0.05, 0.14, 0.19))));
            });
            row.spawn((Text::new(entry.name.clone()), TextFont { font_size: 16.0, ..default() }, TextColor(ui::TEXT), Node { flex_grow: 1.0, ..default() }));
            let mut tag = |text: &str, color: Color| {
                row.spawn((Text::new(text), TextFont { font_size: 12.0, ..default() }, TextColor(color)));
            };
            if entry.id == me {
                tag("YOU", ui::ACCENT);
            }
            if entry.id == 0 {
                tag("HOST", ui::TEXT);
            }
            if show_score {
                if !entry.bot && entry.id != 0 {
                    tag(&format!("{}MS", entry.ping), if entry.ping > 150 { ui::WARN } else { ui::DIM });
                }
                tag(&format!("{} / {}", entry.kills, entry.deaths), ui::TEXT);
            } else if entry.bot {
                tag("BOT", ui::DIM);
            } else if entry.id != 0 || entry.loading {
                if entry.loading {
                    tag("LOADING IN", ui::WARN);
                } else {
                    tag(if entry.ready { "READY" } else { "..." }, if entry.ready { ui::GOOD } else { ui::DIM });
                }
            }
        });
}

fn roster_column(parent: &mut ChildSpawnerCommands, team: u8, state: &MatchState, me: u8, show_score: bool, left: bool) {
    roster_panel(parent, team, state, me, show_score, Some(left), 300.0);
}

/// One team's list. `side` places it at the top left or right of the screen; `None` lets
/// the parent stack it.
fn roster_panel(parent: &mut ChildSpawnerCommands, team: u8, state: &MatchState, me: u8, show_score: bool, side: Option<bool>, width: f32) {
    let mut node = Node {
        position_type: PositionType::Absolute,
        top: Val::Px(110.0),
        width: Val::Px(300.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(5.0),
        padding: UiRect::all(Val::Px(10.0)),
        ..default()
    };
    match side {
        Some(true) => node.left = Val::Px(24.0),
        Some(false) => node.right = Val::Px(24.0),
        None => {
            node.position_type = PositionType::Relative;
            node.top = Val::Auto;
            node.width = Val::Px(width);
        }
    }
    parent.spawn((node, BackgroundColor(Color::srgba(0.02, 0.04, 0.06, 0.8)))).with_children(|column| {
        column.spawn(ui::row(10.0)).with_children(|head| {
            head.spawn((Node { width: Val::Px(14.0), height: Val::Px(14.0), ..default() }, BackgroundColor(TEAM_COLORS[team as usize])));
            ui::label(head, format!("TEAM {}", TEAM_NAMES[team as usize]), 20.0, TEAM_COLORS[team as usize]);
            if show_score {
                ui::label(head, state.scores[team as usize].to_string(), 20.0, ui::TEXT);
            }
        });
        let mut entries: Vec<&RosterEntry> = state.roster.iter().filter(|r| r.team == team).collect();
        if show_score {
            entries.sort_by_key(|r| std::cmp::Reverse(r.kills));
        }
        for entry in entries {
            card(column, entry, me, show_score);
        }
    });
}

/// Where the match is played, as the lobby names it.
fn place(rules: &Rules) -> String {
    use oz_core::worldgen::{Layout, Theme};
    match rules.recipe.layout {
        Layout::Town => format!("{} {}", rules.recipe.biome.look().title, Theme::from_seed(rules.recipe.seed).title()),
        Layout::SmallTown | Layout::MediumTown => format!("{} {}", rules.recipe.biome.look().title, rules.recipe.layout.title()),
        Layout::Archipelago => rules.recipe.biome.look().title.to_string(),
        layout => format!("{} {}", layout.title(), rules.recipe.biome.look().title),
    }
}

fn minutes(seconds: f32) -> String {
    if seconds <= 0.0 { "NONE".to_string() } else { format!("{} MIN", (seconds / 60.0).round()) }
}

/// One row of the lobby's list: a button, or for a guest a value they can only read.
struct List<'a> {
    order: &'a mut Order,
    at: i32,
    host: bool,
}

const WIDTH: f32 = 460.0;

impl List<'_> {
    /// A row of the list: a slim grid button, so the whole lobby fits a 720-line screen.
    fn button(&mut self, list: &mut ChildSpawnerCommands, text: String, act: Act, left: Option<Act>, width: f32, col: i32) {
        ui::slim_at(list, self.order, text, act, left, false, Val::Px(width), Some(IVec2::new(col, self.at)));
    }

    fn item(&mut self, list: &mut ChildSpawnerCommands, text: String, act: Act) {
        self.button(list, text, act, None, WIDTH, 0);
        self.at += 1;
    }

    /// A setting only the host changes: A to edit, left / right to step it.
    fn value(&mut self, list: &mut ChildSpawnerCommands, name: &str, shown: String, step: fn(i8) -> LobbyAction) {
        if self.host {
            self.button(list, format!("{name:<13}<  {shown}  >"), Act::Lobby(step(1)), Some(Act::Lobby(step(-1))), WIDTH, 0);
            self.at += 1;
        } else {
            fixed(list, name, shown);
        }
    }

    /// A host's setting with a button beside it on the same row (the town size and NEW MAP).
    fn value_and(&mut self, list: &mut ChildSpawnerCommands, name: &str, shown: String, step: fn(i8) -> LobbyAction, side: &str, act: Act) {
        if !self.host {
            return self.value(list, name, shown, step);
        }
        const SIDE: f32 = 132.0;
        list.spawn(Node { column_gap: Val::Px(6.0), ..default() }).with_children(|row| {
            self.button(row, format!("{name:<13}<  {shown}  >"), Act::Lobby(step(1)), Some(Act::Lobby(step(-1))), WIDTH - SIDE - 6.0, 0);
            self.button(row, side.to_string(), act, None, SIDE, 1);
        });
        self.at += 1;
    }
}

/// A value nobody can change from here (or only the host can): shown, not focusable.
fn fixed(list: &mut ChildSpawnerCommands, name: &str, shown: String) {
    list.spawn((Node { width: Val::Px(WIDTH), padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)), border: UiRect::all(Val::Px(2.0)), ..default() }, BorderColor(Color::srgba(0.3, 0.4, 0.42, 0.35))))
        .with_children(|row| ui::label(row, format!("{name:<13}{shown}"), 16.0, ui::DIM));
}

/// A small heading between the lobby's groups of rows.
fn heading(list: &mut ChildSpawnerCommands, text: &str) {
    list.spawn(Node { margin: UiRect::top(Val::Px(4.0)), ..default() }).with_children(|row| ui::label(row, text, 12.0, ui::ACCENT));
}

#[allow(clippy::too_many_arguments)]
pub fn build_lobby(
    mut commands: Commands,
    dirty: Res<UiDirty>,
    screen: Res<Screen>,
    controls: Res<Controls>,
    role: Res<NetRole>,
    link: Res<NetLink>,
    rules: Option<Res<Rules>>,
    state: Option<Res<MatchState>>,
    modes: Res<Modes>,
    players: Query<&Player, With<LocalPlayer>>,
    page: Res<LobbyPage>,
    copied: Res<Copied>,
) {
    let (Screen::Lobby, true, Some(rules), Some(state), Ok(me)) = (&*screen, dirty.0, rules, state, players.single()) else {
        return;
    };
    use oz_sim::versus::{choice_size, coop_map_title, coop_mode, is_random_map, map_choice, map_title, random_maps_of_size, skill_name, theme_title};
    let host = *role != NetRole::Client;
    let mode_name = |index: u8| modes.0.get(index as usize).map_or("VERSUS", |m| m.name);
    // A co-op pick (horde) waits here until the host starts it.
    let coop = coop_mode(&modes, state.next_mode);
    // The host's pick shows at once; the lobby reloads on it a moment later.
    let switching = coop.is_none() && (state.next_mode != rules.mode || state.next_map != map_choice(&rules) || state.reroll);
    let next_map_name = coop.map_or_else(|| map_title(state.next_map), |def| coop_map_title(def, state.next_map));
    let mut order = Order::default();
    let n = state.team_size;
    let time = oz_sim::versus::TIME_MODES[(state.time_mode as usize).min(3)];
    let weather = oz_sim::versus::WEATHER_MODES[(state.weather_mode as usize).min(5)];
    let score = if rules.score_limit == 0 { "NONE".to_string() } else { rules.score_limit.to_string() };
    let bots = if rules.no_bots { "OFF - HUMANS ONLY" } else { "FILL EMPTY SLOTS" };
    let skill = skill_name(rules.difficulty);
    let my_name = state.roster.iter().find(|r| r.id == me.id).map_or(me.name.clone(), |r| r.name.clone());

    ui::root(&mut commands, 0.0).with_children(|root| {
        // Top left: what the match is.
        root.spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(18.0),
            left: Val::Px(24.0),
            width: Val::Px(560.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Start,
            row_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|top| {
            if let Some(def) = coop {
                ui::label(top, def.name, 36.0, ui::TEXT);
                ui::label(top, format!("CO-OP   |   {next_map_name}   |   SQUAD OF {n}   |   THREAT {skill}"), 15.0, ui::ACCENT);
                return;
            }
            ui::label(top, mode_name(rules.mode), 36.0, ui::TEXT);
            let limits = match (rules.score_limit, rules.time_limit > 0.0) {
                _ if rules.hunt => format!("SQUAD OF {n}   |   {} HOSTILES   |   {} LIVES", oz_sim::versus::hunt_hostiles(&rules, n), oz_sim::versus::HUNT_LIVES),
                (0, false) => format!("{n} V {n}   |   NO LIMIT"),
                (0, true) => format!("{n} V {n}   |   {}", minutes(rules.time_limit)),
                (limit, false) => format!("{n} V {n}   |   FIRST TO {limit}"),
                (limit, true) => format!("{n} V {n}   |   FIRST TO {limit} IN {}", minutes(rules.time_limit)),
            };
            let timed = if rules.hunt && rules.time_limit > 0.0 { format!("   |   {}", minutes(rules.time_limit)) } else { String::new() };
            ui::label(top, format!("{}   |   {limits}{timed}", place(&rules)), 15.0, ui::ACCENT);
        });
        root.spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(100.0),
            left: Val::Px(24.0),
            width: Val::Px(WIDTH),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Start,
            row_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|bottom| {
            let status = if switching {
                format!("LOADING {}  -  {}...", mode_name(state.next_mode), map_title(state.next_map))
            } else if state.countdown > 0.0 {
                format!("{} STARTS IN {}", if coop.is_some() { mode_name(state.next_mode) } else { "MATCH" }, state.countdown.ceil() as u32)
            } else if coop.is_some() {
                if host { "CO-OP: START TAKES EVERYONE IN. THE LOBBY WAITS FOR YOU.".to_string() } else { "CO-OP: WAITING FOR THE HOST TO START".to_string() }
            } else if page.0 == 1 {
                if host { "MATCH SETTINGS".to_string() } else { "MATCH SETTINGS  (THE HOST CHANGES THESE)".to_string() }
            } else if rules.hunt && host {
                "CLEAR THE TOWN. HOSTILES HOLD IT AND LISTEN FOR YOU: STAY QUIET, USE SUPPRESSORS.".to_string()
            } else if host {
                "START WHEN YOU ARE READY.".to_string()
            } else {
                "WAITING FOR THE HOST TO START".to_string()
            };
            ui::label(bottom, status, 18.0, if switching || state.countdown > 0.0 { ui::GOOD } else { ui::TEXT });
            // One list, the mode first: everything under it is what that mode plays on and
            // uses. Then what you do (start / ready, class, team, name). The match settings
            // are a page of their own. A on a setting starts editing it (left / right).
            bottom.spawn(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(5.0), ..default() }).with_children(|list| {
                let mut rows = List { order: &mut order, at: 0, host };
                if page.0 == 1 {
                    // Only the settings this mode uses.
                    let size = if rules.hunt { format!("{n}") } else { format!("{n} V {n}") };
                    rows.value(list, if rules.hunt { "SQUAD" } else { "TEAMS" }, size, LobbyAction::TeamSize);
                    if !rules.hunt {
                        rows.value(list, if rules.bomb { "ROUNDS TO WIN" } else { "SCORE LIMIT" }, score, LobbyAction::ScoreLimit);
                    }
                    rows.value(list, "TIME LIMIT", minutes(rules.time_limit), LobbyAction::TimeLimit);
                    rows.value(list, if rules.hunt { "SQUAD BOTS" } else { "BOTS" }, bots.to_string(), |_| LobbyAction::Bots);
                    rows.value(list, "BOT SKILL", skill.to_string(), LobbyAction::BotSkill);
                    if !rules.bomb {
                        rows.value(list, "RESPAWN", format!("{} S", rules.respawn_seconds), LobbyAction::Respawn);
                    }
                    rows.value(list, "TIME OF DAY", time.to_string(), LobbyAction::TimeOfDay);
                    rows.value(list, "WEATHER", weather.to_string(), LobbyAction::Weather);
                    rows.item(list, "BACK".to_string(), Act::LobbyPage(0));
                    return;
                }
                let n = state.team_size;
                heading(list, "GAME");
                rows.value(list, "MODE", mode_name(state.next_mode).to_string(), LobbyAction::Mode);
                if let Some(def) = coop {
                    // Co-op modes play on their own set maps.
                    fixed(list, "MAP TYPE", if (def.rules)(0).horde { "SET MAP  (HORDE MAPS)".to_string() } else { "SET MAP".to_string() });
                    rows.value(list, "MAP", next_map_name.to_string(), LobbyAction::Map);
                    rows.value(list, "SQUAD", format!("{n}"), LobbyAction::TeamSize);
                    rows.value(list, "THREAT", skill.to_string(), LobbyAction::BotSkill);
                } else {
                    let random = is_random_map(state.next_map);
                    rows.value(list, "MAP TYPE", if random { "RANDOM TOWN".to_string() } else { "SET MAP".to_string() }, |_| LobbyAction::MapKind);
                    if random {
                        // A random town: its size, a fresh roll, and for the large town a theme.
                        let size = choice_size(state.next_map);
                        rows.value_and(list, "TOWN SIZE", size.title().to_string(), LobbyAction::MapSize, "NEW MAP", Act::Lobby(LobbyAction::NewMap));
                        if random_maps_of_size(size).len() > 1 {
                            rows.value(list, "THEME", theme_title(state.next_map).to_string(), LobbyAction::Map);
                        }
                    } else {
                        rows.value(list, "MAP", format!("{}  -  {}", next_map_name, choice_size(state.next_map).title()), LobbyAction::Map);
                    }
                    rows.value(list, if rules.hunt { "SQUAD" } else { "TEAMS" }, if rules.hunt { format!("{n}") } else { format!("{n} V {n}") }, LobbyAction::TeamSize);
                    rows.item(list, if host { "MATCH SETTINGS".to_string() } else { "VIEW MATCH SETTINGS".to_string() }, Act::LobbyPage(1));
                }
                heading(list, "YOU");
                let ready = state.roster.iter().any(|r| r.id == me.id && r.ready);
                // Only the host starts, and not while a guest is still loading in.
                let loading = state.roster.iter().filter(|r| r.loading).count();
                let first = if host {
                    let start = if coop.is_some() { "START" } else { "START MATCH" };
                    (if loading > 0 { format!("{start}  (WAITING FOR {loading} TO LOAD)") } else { start.to_string() }, LobbyAction::Start)
                } else {
                    ((if ready { "NOT READY" } else { "READY UP" }).to_string(), LobbyAction::ToggleReady)
                };
                rows.item(list, first.0, Act::Lobby(first.1));
                rows.item(list, "ARMORY".to_string(), Act::Go(Screen::Loadout));
                if !rules.hunt && coop.is_none() {
                    rows.item(list, "SWITCH TEAM".to_string(), Act::Lobby(LobbyAction::SwitchTeam));
                }
                rows.item(list, format!("{:<13}{my_name}", "NAME"), Act::EditName);
                rows.item(list, "LEAVE".to_string(), Act::Leave);
            });
            if page.0 == 1 {
                let note = if rules.hunt { "BOT SKILL SETS HOW WELL BOTS AIM AND REACT, AND HOW MANY HOSTILES HOLD THE TOWN." } else { "BOT SKILL SETS HOW WELL BOTS AIM AND REACT." };
                ui::label(bottom, note, 13.0, ui::DIM);
            } else if let Some(mode) = modes.0.get(state.next_mode as usize) {
                ui::label(bottom, mode.blurb, 13.0, ui::DIM);
                let map = if coop.is_some() {
                    "A SET MAP BUILT FOR THIS MODE."
                } else if !is_random_map(state.next_map) {
                    "SET MAP: HAND-BUILT, THE SAME STREETS EVERY MATCH."
                } else if state.next_map == 0 {
                    "RANDOM TOWN: NEW STREETS AND A ROLLED THEME EVERY TIME."
                } else {
                    "RANDOM TOWN: NEW STREETS EVERY TIME."
                };
                ui::label(bottom, map, 13.0, ui::ACCENT);
            }
            if let NetLink::Host(host) = &*link {
                ui::label(bottom, host_address(host), 13.0, ui::ACCENT);
            }
        });
        // How to drive it, out of the way in the bottom right.
        root.spawn(Node { position_type: PositionType::Absolute, bottom: Val::Px(10.0), right: Val::Px(24.0), ..default() }).with_children(|corner| {
            let hint = if controls.gamepad { "D-PAD / STICK MOVES   (A) SELECT   (A) ON A SETTING, THEN LEFT / RIGHT" } else { "CLICK  <  >  TO CHANGE A SETTING, OR ARROWS + ENTER" };
            ui::label(corner, hint, 12.0, ui::DIM);
        });
        // Right: the lobby code on top, big and a click to copy; under it everyone here, both
        // teams side by side (a co-op squad or a hunt's squad is one team).
        root.spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(14.0),
            right: Val::Px(24.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::End,
            row_gap: Val::Px(10.0),
            ..default()
        })
        .with_children(|right| {
            if let Some(code) = lobby_code(&link) {
                let (hint, tint) = match copied.0 {
                    Some((true, _)) => ("COPIED - PASTE IT INTO JOIN", ui::GOOD),
                    Some((false, _)) => ("COULD NOT COPY - READ IT OUT", ui::WARN),
                    None => ("CLICK TO COPY", ui::DIM),
                };
                ui::code_button(right, &mut order, Act::CopyCode, "LOBBY CODE", &spaced_code(&code), hint, tint, Val::Px(612.0), IVec2::new(0, -1));
            }
            right.spawn(Node { column_gap: Val::Px(12.0), align_items: AlignItems::Start, ..default() }).with_children(|teams| {
                let one_team = rules.hunt || coop.is_some();
                roster_panel(teams, 0, &state, me.id, false, None, if one_team { 612.0 } else { 300.0 });
                if !one_team {
                    roster_panel(teams, 1, &state, me.id, false, None, 300.0);
                }
            });
        });

    });
}

/// The in-match scoreboard while Tab / Back is held, and on the post-match screen.
fn scoreboard(
    mut commands: Commands,
    controls: Res<Controls>,
    screen: Res<Screen>,
    rules: Res<Rules>,
    state: Res<MatchState>,
    players: Query<&Player, With<LocalPlayer>>,
    existing: Query<Entity, With<Scoreboard>>,
    mut shown: Local<Option<(Vec<RosterEntry>, [u32; 2])>>,
) {
    let visible = rules.pvp && ((controls.scoreboard && *screen == Screen::None) || (*screen == Screen::GameOver && state.phase == Phase::Won));
    let signature = (state.roster.clone(), state.scores);
    if !visible {
        if shown.take().is_some() {
            for entity in &existing {
                commands.entity(entity).despawn();
            }
        }
        return;
    }
    if shown.as_ref() == Some(&signature) {
        return;
    }
    *shown = Some(signature);
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let me = players.single().map_or(255, |p| p.id);
    let results = *screen == Screen::GameOver;
    commands.spawn((Scoreboard, Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, GlobalZIndex(20))).with_children(|root| {
        if results {
            // The results panel has the left: both teams stack on the right.
            root.spawn(Node { position_type: PositionType::Absolute, top: Val::Px(110.0), right: Val::Px(24.0), flex_direction: FlexDirection::Column, row_gap: Val::Px(12.0), ..default() }).with_children(|teams| {
                roster_panel(teams, 0, &state, me, true, None, 340.0);
                if !rules.hunt {
                    roster_panel(teams, 1, &state, me, true, None, 340.0);
                }
            });
            return;
        }
        roster_column(root, 0, &state, me, true, true);
        if !rules.hunt {
            roster_column(root, 1, &state, me, true, false);
        }
    });
}
