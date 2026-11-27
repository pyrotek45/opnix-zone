//! The versus lobby and the scoreboard.
//!
//! The lobby shows both teams as roster cards around your squad standing on
//! the showroom stage, with ready / start, team switching, team size and the
//! class editor. Everything is a `LobbyAction` command, so it works the same
//! for the host, for guests and offline. Holding Tab (Back on a pad) during
//! a match shows the scoreboard.

use crate::{
    ClientSet,
    input::Controls,
    ui::{self, Act, ActEvent, Order, Screen, UiDirty},
};
use bevy::{ecs::hierarchy::ChildSpawnerCommands, prelude::*};
use oz_sim::prelude::*;

/// What to tell friends: the LAN address, and the internet address if the router opened the port.
pub fn hosting_info(host: &oz_sim::net::HostLink) -> String {
    use oz_net::lan::{PortStatus, local_ip};
    let lan = local_ip().map_or("NO NETWORK".to_string(), |ip| format!("{ip}:{}", host.port));
    let internet = match (host.router_status(), host.public_addr()) {
        (Some(PortStatus::Open(ip)), _) => format!("DIRECT  {ip}:{}", host.port),
        (_, Some(public)) => format!("DIRECT  {public}"),
        (Some(PortStatus::Trying), None) => "FINDING INTERNET ADDRESS...".to_string(),
        (Some(_), None) => "NO INTERNET ADDRESS YET".to_string(),
        (None, None) => "LOCAL ONLY".to_string(),
    };
    let code = host.join_code().map_or(String::new(), |code| format!("JOIN CODE  {}      ", oz_net::rendezvous::pretty_code(&code)));
    format!("{code}LAN  {lan}      {internet}")
}

pub const TEAM_NAMES: [&str; 2] = ["TEAL", "CORAL"];
pub const TEAM_COLORS: [Color; 2] = [Color::srgb(0.25, 0.8, 0.82), Color::srgb(0.95, 0.52, 0.42)];

#[derive(Component)]
struct Scoreboard;

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, lobby_actions.in_set(ClientSet::Input).after(crate::input::ReadDevices).before(crate::menu::MenuActions).run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, (refresh_lobby.before(crate::menu::BuildScreens), scoreboard).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

fn lobby_actions(mut events: EventReader<ActEvent>, screen: Res<Screen>, players: Query<&Player, With<LocalPlayer>>, mut orders: EventWriter<CommandEvent>) {
    if !matches!(*screen, Screen::Lobby | Screen::GameOver) {
        return;
    }
    let Ok(player) = players.single() else { return };
    for ActEvent(act) in events.read() {
        if let Act::Lobby(action) = act {
            orders.write(CommandEvent { player: player.id, command: PlayerCommand::Lobby(*action) });
        }
    }
}

/// Rebuild the lobby whenever anything on it changes.
fn refresh_lobby(screen: Res<Screen>, state: Res<MatchState>, link: Res<NetLink>, mut dirty: ResMut<UiDirty>, mut last: Local<Option<(Vec<RosterEntry>, u8, i32, Phase, String, (u8, u8))>>) {
    if *screen != Screen::Lobby {
        *last = None;
        return;
    }
    let router = match &*link {
        NetLink::Host(host) => format!("{:?}", host.router_status()),
        _ => String::new(),
    };
    // Ping changes constantly; leave it out so the lobby does not rebuild every frame.
    let roster: Vec<RosterEntry> = state.roster.iter().map(|r| RosterEntry { ping: 0, weapon: 0, parts: Default::default(), ..r.clone() }).collect();
    let signature = (roster, state.team_size, state.countdown.ceil() as i32, state.phase, router, (state.time_mode, state.weather_mode));
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
            } else {
                tag(if entry.ready { "READY" } else { "..." }, if entry.ready { ui::GOOD } else { ui::DIM });
            }
        });
}

fn roster_column(parent: &mut ChildSpawnerCommands, team: u8, state: &MatchState, me: u8, show_score: bool, left: bool) {
    let mut node = Node {
        position_type: PositionType::Absolute,
        top: Val::Px(110.0),
        width: Val::Px(300.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(5.0),
        padding: UiRect::all(Val::Px(10.0)),
        ..default()
    };
    if left {
        node.left = Val::Px(24.0);
    } else {
        node.right = Val::Px(24.0);
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
) {
    let (Screen::Lobby, true, Some(rules), Some(state), Ok(me)) = (&*screen, dirty.0, rules, state, players.single()) else {
        return;
    };
    let host = *role != NetRole::Client;
    let mode = modes.0.get(rules.mode as usize).map_or("VERSUS", |m| m.name);
    let mut order = Order::default();
    let order = &mut order;

    ui::root(&mut commands, 0.0).with_children(|root| {
        root.spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(18.0),
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|top| {
            ui::label(top, mode, 34.0, ui::TEXT);
            let n = state.team_size;
            let place = match rules.recipe.layout {
                oz_core::worldgen::Layout::Town => {
                    format!("{} TOWN", rules.recipe.biome.look().title)
                }
                layout @ (oz_core::worldgen::Layout::Crossroads | oz_core::worldgen::Layout::Dockyard) => layout.title().to_string(),
                _ => rules.recipe.biome.look().title.to_string(),
            };
            let line = if rules.hunt {
                format!("{place}   |   SQUAD OF {n}   |   {} HOSTILES   |   {} LIVES", oz_sim::versus::hunt_hostiles(&rules, n), oz_sim::versus::HUNT_LIVES)
            } else {
                format!("{place}   |   {n} V {n}   |   FIRST TO {}", rules.score_limit)
            };
            ui::label(top, line, 15.0, ui::ACCENT);
        });
        roster_column(root, 0, &state, me.id, false, true);
        if !rules.hunt {
            roster_column(root, 1, &state, me.id, false, false);
        }

        root.spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(20.0),
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|bottom| {
            let status = if rules.hunt && state.countdown <= 0.0 && host {
                "CLEAR THE TOWN. HOSTILES HOLD IT AND LISTEN FOR YOU: STAY QUIET, USE SUPPRESSORS.".to_string()
            } else if state.countdown > 0.0 {
                format!("MATCH STARTS IN {}", state.countdown.ceil() as u32)
            } else if host {
                "START WHEN YOU ARE READY. BOTS FILL EVERY EMPTY SLOT.".to_string()
            } else {
                "WAITING FOR THE HOST TO START".to_string()
            };
            ui::label(bottom, status, 18.0, if state.countdown > 0.0 { ui::GOOD } else { ui::TEXT });
            // Actions in one row (d-pad left / right moves along it), settings in the row below
            // (left / right changes the focused one, up / down moves between the rows).
            bottom.spawn(ui::row(8.0)).with_children(|row| {
                if host {
                    ui::sized_button(row, order, "START MATCH", Act::Lobby(LobbyAction::Start), None, Val::Px(190.0));
                } else {
                    let ready = state.roster.iter().any(|r| r.id == me.id && r.ready);
                    ui::sized_button(row, order, if ready { "NOT READY" } else { "READY UP" }, Act::Lobby(LobbyAction::ToggleReady), None, Val::Px(190.0));
                }
                ui::sized_button(row, order, "ARMORY", Act::Go(Screen::Loadout), None, Val::Px(170.0));
                if !rules.hunt {
                    ui::sized_button(row, order, "SWITCH TEAM", Act::Lobby(LobbyAction::SwitchTeam), None, Val::Px(170.0));
                }
                ui::sized_button(row, order, "LEAVE", Act::Leave, None, Val::Px(110.0));
            });
            // Match settings: the host steps them, everyone sees them.
            let time = oz_sim::versus::TIME_MODES[(state.time_mode as usize).min(3)];
            let weather = oz_sim::versus::WEATHER_MODES[(state.weather_mode as usize).min(5)];
            let n = state.team_size;
            let size = if rules.hunt { format!("SQUAD  <  {n}  >") } else { format!("TEAMS  <  {n} V {n}  >") };
            if host {
                bottom.spawn(ui::row(8.0)).with_children(|row| {
                    ui::sized_button(row, order, size, Act::Lobby(LobbyAction::TeamSize(1)), Some(Act::Lobby(LobbyAction::TeamSize(-1))), Val::Px(240.0));
                    ui::sized_button(row, order, format!("TIME  <  {time}  >"), Act::Lobby(LobbyAction::TimeOfDay(1)), Some(Act::Lobby(LobbyAction::TimeOfDay(-1))), Val::Px(300.0));
                    ui::sized_button(row, order, format!("WEATHER  <  {weather}  >"), Act::Lobby(LobbyAction::Weather(1)), Some(Act::Lobby(LobbyAction::Weather(-1))), Val::Px(300.0));
                });
            } else {
                ui::label(bottom, format!("TIME: {time}      WEATHER: {weather}"), 14.0, ui::ACCENT);
            }
            if let NetLink::Host(host) = &*link {
                ui::label(bottom, hosting_info(host), 13.0, ui::ACCENT);
            }
            let hint = if controls.gamepad { "D-PAD / STICK MOVES   (A) SELECT   LEFT / RIGHT ON A SETTING CHANGES IT" } else { "CLICK, OR ARROWS + ENTER.  LEFT / RIGHT ON A SETTING CHANGES IT." };
            ui::label(bottom, hint, 12.0, ui::DIM);
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
    commands.spawn((Scoreboard, Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, GlobalZIndex(20))).with_children(|root| {
        roster_column(root, 0, &state, me, true, true);
        if !rules.hunt {
            roster_column(root, 1, &state, me, true, false);
        }
    });
}
