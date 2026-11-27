//! Feel-good feedback on top of the plain effects: medal banners for your
//! kills (double kill, headshot, revenge, first blood...), killstreak
//! fanfares and call-in announcements, a heartbeat when you are nearly
//! dead, and a little punch of the view on every kill. Also sends the
//! killstreak call-in command when its key is pressed.

use crate::{
    ClientSet,
    fx::{DeathInfo, PlaySfx},
    input::Controls,
    ui::{self, Screen},
    view::ViewState,
};
use bevy::prelude::*;
use oz_sim::{killstreak, prelude::*};
use std::collections::VecDeque;

/// Seconds between kills that still count as one multi-kill.
const CHAIN: f32 = 4.0;
const BANNER_SECONDS: f32 = 1.9;

#[derive(Resource, Default)]
pub struct Banners {
    queue: VecDeque<(String, String, Color)>,
    showing: Option<f32>,
}

impl Banners {
    pub fn push(&mut self, title: impl Into<String>, subtitle: impl Into<String>, color: Color) {
        // Never let a burst of medals back up for long.
        if self.queue.len() < 4 {
            self.queue.push_back((title.into(), subtitle.into(), color));
        }
    }
}

#[derive(Component)]
struct BannerRoot;
#[derive(Component)]
struct BannerTitle;
#[derive(Component)]
struct BannerSubtitle;

/// Your recent kill times, for multi-kills, and whether anyone has scored yet.
#[derive(Resource, Default)]
struct KillMemory {
    times: Vec<f32>,
    first_blood: bool,
    heartbeat: f32,
}

pub struct JuicePlugin;

/// A line for the announcer to say (see `announcer`; silent without a voice installed).
#[derive(Event, Clone, Debug)]
pub struct Announce(pub String);

impl Plugin for JuicePlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<Announce>()
            .init_resource::<Banners>()
            .init_resource::<KillMemory>()
            .add_systems(
                OnEnter(MatchPhase::Active),
                (spawn_banner, |mut memory: ResMut<KillMemory>, mut banners: ResMut<Banners>| {
                    *memory = KillMemory::default();
                    *banners = Banners::default();
                }),
            )
            .add_systems(Update, call_in.in_set(ClientSet::Input).after(crate::input::ReadDevices).run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, (medals, heartbeat, show_banner).chain().in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

fn call_in(controls: Res<Controls>, screen: Res<Screen>, players: Query<&Player, With<LocalPlayer>>, mut orders: EventWriter<CommandEvent>, mut sounds: EventWriter<PlaySfx>) {
    if !controls.killstreak || *screen != Screen::None {
        return;
    }
    let Ok(player) = players.single() else { return };
    if oz_sim::killstreak::next(&player.rewards).is_none() {
        sounds.write(PlaySfx::ui(Sfx::UiDeny));
        return;
    }
    orders.write(CommandEvent { player: player.id, command: PlayerCommand::UseStreak });
}

fn spawn_banner(mut commands: Commands) {
    commands
        .spawn((
            MatchEntity,
            BannerRoot,
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), top: Val::Percent(24.0), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, ..default() },
            Visibility::Hidden,
            GlobalZIndex(30),
        ))
        .with_children(|root| {
            root.spawn((BannerTitle, Text::new(""), TextFont { font_size: 40.0, ..default() }, TextColor(ui::TEXT), TextShadow::default()));
            root.spawn((BannerSubtitle, Text::new(""), TextFont { font_size: 17.0, ..default() }, TextColor(ui::ACCENT), TextShadow::default()));
        });
}

#[allow(clippy::too_many_arguments)]
fn medals(
    time: Res<Time>,
    mut events: EventReader<FxEvent>,
    state: Res<MatchState>,
    rules: Res<Rules>,
    death: Res<DeathInfo>,
    mut memory: ResMut<KillMemory>,
    mut banners: ResMut<Banners>,
    mut view: ResMut<ViewState>,
    mut sounds: EventWriter<PlaySfx>,
    mut notices: ResMut<crate::hud::Notices>,
    local: Query<&Player, With<LocalPlayer>>,
    mut announce: EventWriter<Announce>,
) {
    let Ok(me) = local.single() else {
        events.clear();
        return;
    };
    let now = time.elapsed_secs();
    let name_of = |id: u8| state.roster.iter().find(|r| r.id == id).map_or("SOMEONE".to_string(), |r| r.name.clone());
    for event in events.read() {
        match event {
            FxEvent::Killed { victim, by, head, .. } => {
                let first = !memory.first_blood;
                memory.first_blood = true;
                if *by != me.id || *victim == me.id {
                    continue;
                }
                view.punch = 1.0;
                memory.times.retain(|t| now - t < CHAIN);
                memory.times.push(now);
                let chain = memory.times.len();
                let mut titles: Vec<&str> = Vec::new();
                match chain {
                    2 => titles.push("DOUBLE KILL"),
                    3 => titles.push("TRIPLE KILL"),
                    4 => titles.push("QUAD KILL"),
                    n if n >= 5 => titles.push("RAMPAGE"),
                    _ => {}
                }
                if *head {
                    titles.push("HEADSHOT");
                    sounds.write(PlaySfx::new(Sfx::Headshot, None));
                }
                if death.killer == Some(*victim) {
                    titles.push("REVENGE");
                }
                if first && rules.pvp {
                    titles.push("FIRST BLOOD");
                }
                let subtitle = format!("{}  +{}", name_of(*victim), 100 + 50 * titles.len());
                if let Some((title, rest)) = titles.split_first() {
                    let more = rest.join("  +  ");
                    banners.push(*title, if more.is_empty() { subtitle } else { format!("{more}   {subtitle}") }, ui::GOOD);
                    sounds.write(PlaySfx::new(Sfx::Medal, None));
                } else {
                    banners.push("", subtitle, ui::TEXT);
                }
            }
            FxEvent::LimbLost { player, limb, .. } if *player == me.id => {
                use oz_sim::player::limb as l;
                let (what, effect) = match *limb {
                    l::LEFT_ARM => ("LEFT ARM", "AIM UNSTEADY"),
                    l::RIGHT_ARM => ("RIGHT ARM", "AIM UNSTEADY"),
                    l::LEFT_LEG => ("LEFT LEG", "MOVING SLOWER"),
                    _ => ("RIGHT LEG", "MOVING SLOWER"),
                };
                banners.push(format!("{what} DESTROYED"), effect, Color::srgb(1.0, 0.45, 0.3));
                sounds.write(PlaySfx::new(Sfx::Alarm, None));
            }
            FxEvent::Streak { player, reward } if *player == me.id => {
                let def = killstreak::def(*reward);
                let kills = def.map_or(0, |d| d.kills);
                banners.push(format!("{kills} KILL STREAK"), format!("{} READY", killstreak::name(*reward)), Color::srgb(1.0, 0.8, 0.35));
                sounds.write(PlaySfx::new(Sfx::Streak, None));
            }
            FxEvent::StreakUsed { name, team, reward } => {
                let ours = *team == me.team;
                let what = killstreak::name(*reward);
                if ours {
                    notices.push(format!("{name}: {what} INCOMING"));
                    sounds.write(PlaySfx::new(if *reward == killstreak::reward::UAV { Sfx::Radar } else { Sfx::UiAccept }, None));
                    let line = match *reward {
                        killstreak::reward::UAV => "Radar plane is up.",
                        killstreak::reward::AIRSTRIKE => "Airstrike inbound.",
                        killstreak::reward::ARTILLERY => "Artillery on the way.",
                        killstreak::reward::SUPPLY => "Supply drop. Rearm.",
                        _ => "Attack drone deployed.",
                    };
                    announce.write(Announce(line.into()));
                } else {
                    // What is coming, and what to do about it: aircraft can be shot down.
                    let (warning, advice, line) = match *reward {
                        killstreak::reward::UAV => ("ENEMY RADAR PLANE SPOTTED", "SHOOT IT DOWN TO GO DARK ON THEIR RADAR", "Enemy radar plane spotted."),
                        killstreak::reward::AIRSTRIKE => ("ENEMY BOMBER INBOUND", "SHOOT IT DOWN OR GET UNDER COVER", "Enemy bomber inbound."),
                        killstreak::reward::ARTILLERY => ("INCOMING ARTILLERY", "GET INDOORS", "Incoming artillery. Get indoors."),
                        killstreak::reward::SUPPLY => ("ENEMY SUPPLY DROP", "THEY JUST REARMED", "Enemy supply drop."),
                        _ => ("ENEMY ATTACK DRONE", "SHOOT IT DOWN", "Enemy attack drone. Shoot it down."),
                    };
                    banners.push(warning, advice, Color::srgb(1.0, 0.35, 0.3));
                    sounds.write(PlaySfx::new(Sfx::Alarm, None));
                    announce.write(Announce(line.into()));
                }
            }
            _ => {}
        }
    }
}

/// Thump thump when you are nearly dead.
fn heartbeat(time: Res<Time>, mut memory: ResMut<KillMemory>, players: Query<&Health, With<LocalPlayer>>, mut sounds: EventWriter<PlaySfx>) {
    let Ok(health) = players.single() else { return };
    memory.heartbeat -= time.delta_secs();
    if health.alive() && health.fraction() < 0.3 && memory.heartbeat <= 0.0 {
        // Faster the closer to death.
        memory.heartbeat = 0.6 + health.fraction() * 1.5;
        sounds.write(PlaySfx { volume: 0.8, ..PlaySfx::new(Sfx::Heartbeat, None) });
    }
}

#[allow(clippy::type_complexity)]
fn show_banner(
    time: Res<Time>,
    mut banners: ResMut<Banners>,
    mut root: Query<(&mut Visibility, &mut Transform), With<BannerRoot>>,
    mut title: Query<(&mut Text, &mut TextColor), (With<BannerTitle>, Without<BannerSubtitle>)>,
    mut subtitle: Query<&mut Text, (With<BannerSubtitle>, Without<BannerTitle>)>,
) {
    let (Ok((mut visibility, mut transform)), Ok((mut title_text, mut title_color)), Ok(mut subtitle_text)) = (root.single_mut(), title.single_mut(), subtitle.single_mut()) else {
        return;
    };
    let dt = time.delta_secs();
    match banners.showing {
        Some(age) if age < BANNER_SECONDS => {
            let age = age + dt;
            banners.showing = Some(age);
            // Slam in big, settle, then fade away.
            let slam = 1.0 + 0.7 * (1.0 - (age / 0.14).min(1.0)).powi(2);
            transform.scale = Vec3::splat(slam);
            let fade = ((BANNER_SECONDS - age) / 0.35).clamp(0.0, 1.0);
            title_color.0 = title_color.0.with_alpha(fade);
        }
        _ => {
            let Some((title, sub, color)) = banners.queue.pop_front() else {
                banners.showing = None;
                if *visibility != Visibility::Hidden {
                    *visibility = Visibility::Hidden;
                }
                return;
            };
            title_text.0 = title;
            title_color.0 = color;
            subtitle_text.0 = sub;
            banners.showing = Some(0.0);
            *visibility = Visibility::Inherited;
        }
    }
}
