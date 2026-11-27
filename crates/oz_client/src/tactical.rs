//! Tactical HUD and hand state.
//!
//! * The killstreak stack (bottom right, above the ammo): up to three earned
//!   rewards, newest on top - that is the one the call-in button uses - and
//!   how many more kills the next one needs.
//! * The grenade in your hand: a fuse meter while it cooks, with the keys to
//!   throw it or put the pin back. The same state drives the first-person
//!   throwing arm (`view`).
//! * The scope's stabilizer (servo lock) meter.
//! * A vault prompt when there is a wall, sill or crate in front to go over.
//! * Terrorist hunt: hostiles left and the squad's lives.

use crate::{
    ClientSet,
    bindings::Action,
    input::{Controls, Steady},
    settings::Settings,
    ui::{self, Screen},
};
use bevy::prelude::*;
use oz_sim::{
    killstreak::{self, STACK},
    prelude::*,
};

/// What the local player's hands are doing besides holding the gun.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct Hands {
    /// Seconds the grenade in hand has been cooking, and its fuse.
    pub cooking: Option<f32>,
    pub fuse: f32,
    /// Whether the held gear goes off when the fuse runs out (frags do, knives do not).
    pub live: bool,
    /// Seconds since the last throw, pin-back or melee (for the arm animations).
    pub since_throw: f32,
    pub since_cancel: f32,
    pub since_melee: f32,
}

#[derive(Component)]
struct StreakSlot(usize);

#[derive(Component)]
struct StreakText;

#[derive(Component)]
struct StreakPanel;

#[derive(Component)]
struct HandMeter;

#[derive(Component)]
struct HandFill;

#[derive(Component)]
struct HandText;

#[derive(Component)]
struct HuntText;

pub struct TacticalPlugin;

impl Plugin for TacticalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hands>()
            .add_systems(OnEnter(MatchPhase::Active), (spawn, |mut hands: ResMut<Hands>| *hands = Hands { since_throw: 9.0, since_cancel: 9.0, since_melee: 9.0, ..default() }))
            .add_systems(Update, track_hands.in_set(ClientSet::Input).after(crate::input::DrivePlayer).run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, update.in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

/// The fuse of the local player's gear, if it is something thrown and held.
pub fn held_gear(content: &Content, player: &Player) -> Option<(f32, bool)> {
    let gear = content.gear.get(player.gear as usize)?;
    let (_, shot) = oz_sim::combat::throw(&gear.kind, Vec3::NEG_Z)?;
    let knife = matches!(gear.kind, GearKind::Knife { .. });
    Some((shot.life, !knife && matches!(shot.effect, oz_sim::combat::Impact::Blast)))
}

/// Mirrors the authority's grenade rules locally so the hand and meter react at once online.
fn track_hands(
    time: Res<Time>,
    controls: Res<Controls>,
    screen: Res<Screen>,
    content: Res<Content>,
    state: Res<MatchState>,
    mut hands: ResMut<Hands>,
    players: Query<(&Player, &PlayerInput, &Health), With<LocalPlayer>>,
    mut was_held: Local<bool>,
) {
    let dt = time.delta_secs();
    hands.since_throw += dt;
    hands.since_cancel += dt;
    hands.since_melee += dt;
    let Ok((player, input, health)) = players.single() else {
        return;
    };
    let held = input.held(buttons::GEAR) && *screen == Screen::None && health.alive();
    let pressed = held && !*was_held;
    *was_held = held;
    if input.held(buttons::MELEE) && hands.cooking.is_none() && hands.since_melee > 0.7 {
        hands.since_melee = 0.0;
    }
    if !health.alive() || state.phase == Phase::Lobby {
        hands.cooking = None;
        return;
    }
    if pressed
        && hands.cooking.is_none()
        && player.gear_left > 0
        && let Some((fuse, live)) = held_gear(&content, player)
    {
        hands.cooking = Some(0.0);
        hands.fuse = fuse;
        hands.live = live;
    }
    if let Some(cooked) = hands.cooking {
        if controls.reload_pressed {
            hands.cooking = None;
            hands.since_cancel = 0.0;
        } else if !held || (hands.live && cooked + dt >= hands.fuse) {
            hands.cooking = None;
            hands.since_throw = 0.0;
        } else {
            hands.cooking = Some(cooked + dt);
        }
    }
}

fn spawn(mut commands: Commands) {
    // Killstreaks: bottom right, above the ammo counter.
    commands
        .spawn((
            MatchEntity,
            StreakPanel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(20.0),
                bottom: Val::Px(128.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::End,
                row_gap: Val::Px(3.0),
                ..default()
            },
            GlobalZIndex(-40),
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            for slot in 0..STACK {
                panel
                    .spawn((
                        StreakSlot(slot),
                        Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)), border: UiRect::left(Val::Px(3.0)), min_width: Val::Px(170.0), justify_content: JustifyContent::End, ..default() },
                        BackgroundColor(Color::srgba(0.02, 0.04, 0.06, 0.7)),
                        BorderColor(Color::srgba(0.3, 0.4, 0.42, 0.5)),
                    ))
                    .with_children(|row| {
                        row.spawn((Text::new(""), TextFont { font_size: 13.0, ..default() }, TextColor(ui::TEXT)));
                    });
            }
            panel.spawn((StreakText, Text::new(""), TextFont { font_size: 11.0, ..default() }, TextColor(ui::DIM), TextLayout::new_with_justify(JustifyText::Right)));
        });

    // The meter under the crosshair: grenade fuse or scope stabilizer.
    commands
        .spawn((
            MatchEntity,
            HandMeter,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                top: Val::Percent(56.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(3.0),
                ..default()
            },
            GlobalZIndex(-38),
            Visibility::Hidden,
        ))
        .with_children(|meter| {
            meter.spawn((Node { width: Val::Px(150.0), height: Val::Px(6.0), ..default() }, BackgroundColor(Color::srgba(0.1, 0.14, 0.17, 0.85)))).with_children(|track| {
                track.spawn((HandFill, Node { height: Val::Percent(100.0), width: Val::Percent(0.0), ..default() }, BackgroundColor(ui::TEXT)));
            });
            meter.spawn((HandText, Text::new(""), TextFont { font_size: 12.0, ..default() }, TextColor(ui::TEXT), TextShadow::default()));
        });

    // Terrorist hunt status, under the score bar.
    commands.spawn((
        MatchEntity,
        HuntText,
        Text::new(""),
        TextFont { font_size: 15.0, ..default() },
        TextColor(ui::TEXT),
        TextShadow::default(),
        TextLayout::new_with_justify(JustifyText::Center),
        Node { position_type: PositionType::Absolute, top: Val::Px(50.0), width: Val::Percent(100.0), justify_content: JustifyContent::Center, ..default() },
        GlobalZIndex(-38),
    ));
}

#[allow(clippy::too_many_arguments)]
fn update(
    controls: Res<Controls>,
    settings: Res<Settings>,
    screen: Res<Screen>,
    rules: Res<Rules>,
    state: Res<MatchState>,
    hands: Res<Hands>,
    steady: Res<Steady>,
    view: Res<crate::view::ViewState>,
    (map, statics): (Res<Map>, Res<Statics>),
    players: Query<(&Player, &Transform, &PlayerInput, &Health, &Mover), With<LocalPlayer>>,
    mut panel: Query<&mut Visibility, (With<StreakPanel>, Without<HandMeter>)>,
    mut slots: Query<(&StreakSlot, &Children, &mut BackgroundColor, &mut BorderColor)>,
    mut texts: Query<&mut Text, (Without<StreakText>, Without<HandText>, Without<HuntText>)>,
    mut streak_text: Query<&mut Text, (With<StreakText>, Without<HandText>, Without<HuntText>)>,
    (mut meter, mut fill, mut hand_text, mut hunt_text): (
        Query<&mut Visibility, (With<HandMeter>, Without<StreakPanel>)>,
        Query<(&mut Node, &mut BackgroundColor), (With<HandFill>, Without<StreakSlot>)>,
        Query<&mut Text, (With<HandText>, Without<StreakText>, Without<HuntText>)>,
        Query<&mut Text, (With<HuntText>, Without<StreakText>, Without<HandText>)>,
    ),
) {
    let Ok((player, transform, input, health, mover)) = players.single() else {
        return;
    };
    let playing = *screen == Screen::None && health.alive();
    let key = |action: Action| settings.bindings.label(action);
    let set = |text: &mut Text, value: String| {
        if text.0 != value {
            text.0 = value;
        }
    };

    // Killstreaks.
    let show_streaks = rules.pvp && playing && state.phase == Phase::Live;
    if let Ok(mut visibility) = panel.single_mut() {
        let wanted = if show_streaks { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    if show_streaks {
        // Newest on top: that is the one the button calls in.
        let stack: Vec<u8> = player.rewards.iter().rev().copied().filter(|r| *r != 0).collect();
        let call = if controls.gamepad { "D-PAD >".to_string() } else { key(Action::Killstreak) };
        for (slot, children, mut background, mut border) in &mut slots {
            let entry = stack.get(slot.0).copied();
            let line = match entry {
                Some(id) if slot.0 == 0 => format!("[{call}]  {}", killstreak::name(id)),
                Some(id) => killstreak::name(id).to_string(),
                None => "-".to_string(),
            };
            if let Some(mut text) = children.first().and_then(|c| texts.get_mut(*c).ok()) {
                set(&mut text, line);
            }
            let (fill, edge) = match entry {
                Some(_) if slot.0 == 0 => (Color::srgba(0.35, 0.28, 0.08, 0.9), Color::srgb(1.0, 0.8, 0.35)),
                Some(_) => (Color::srgba(0.06, 0.1, 0.13, 0.85), ui::ACCENT),
                None => (Color::srgba(0.02, 0.04, 0.06, 0.5), Color::srgba(0.3, 0.4, 0.42, 0.4)),
            };
            if background.0 != fill {
                background.0 = fill;
                border.0 = edge;
            }
        }
        let next = player.streaks.iter().filter_map(|id| killstreak::def(*id)).find(|d| d.kills > player.streak);
        let progress = match next {
            Some(def) => format!("STREAK {}   NEXT: {} IN {}", player.streak, def.name, def.kills - player.streak),
            None => format!("STREAK {}", player.streak),
        };
        if let Ok(mut text) = streak_text.single_mut() {
            set(&mut text, progress);
        }
    }

    // The meter under the crosshair.
    let pin_key = if controls.gamepad { "X".to_string() } else { key(Action::Reload) };
    let (shown, fraction, color, line) = if let Some(cooked) = hands.cooking.filter(|_| playing) {
        let left = (hands.fuse - cooked).max(0.0);
        let fraction = if hands.live { left / hands.fuse.max(0.01) } else { 1.0 };
        let color = if hands.live && left < 0.6 { ui::WARN } else { Color::srgb(1.0, 0.8, 0.35) };
        let line = if hands.live { format!("COOKING  {left:.1}s   RELEASE: THROW   [{pin_key}] PIN BACK") } else { format!("RELEASE: THROW   [{pin_key}] PUT AWAY") };
        (true, fraction, color, line)
    } else if view.scoped > 0.5 && playing {
        let stab = if controls.gamepad { "L3".to_string() } else { key(Action::Sprint) };
        if steady.left > 0.0 {
            (true, steady.left / steady.total.max(0.01), ui::GOOD, "STABILIZED".to_string())
        } else if steady.cooldown > 0.0 {
            (true, 1.0 - steady.cooldown / 2.0, ui::DIM, "SERVOS RECOVERING".to_string())
        } else {
            (true, 1.0, ui::ACCENT, format!("[{stab}] STABILIZE  ({:.0}s)", Steady::duration(input.stance)))
        }
    } else if playing && mover.grounded && input.stance != stance::PRONE && mover.vault.is_none() {
        // Something low ahead to go over?
        let wish = Quat::from_rotation_y(input.yaw) * Vec3::new(controls.movement.x, 0.0, -controls.movement.y);
        let dir = if wish.length() > 0.2 { wish } else { Quat::from_rotation_y(input.yaw) * Vec3::NEG_Z };
        let ahead = oz_sim::player::vault_target(&map, &statics, transform.translation, dir).is_some()
            && statics.raycast(transform.translation + Vec3::Y * 0.35, Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero(), 0.9).is_some();
        let jump = if controls.gamepad { "A".to_string() } else { key(Action::Jump) };
        (ahead, 0.0, ui::ACCENT, format!("[{jump}] VAULT"))
    } else {
        (false, 0.0, ui::TEXT, String::new())
    };
    if let Ok(mut visibility) = meter.single_mut() {
        let wanted = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    if let Ok((mut node, mut background)) = fill.single_mut() {
        let width = Val::Percent(fraction.clamp(0.0, 1.0) * 100.0);
        if node.width != width {
            node.width = width;
        }
        if background.0 != color {
            background.0 = color;
        }
    }
    if let Ok(mut text) = hand_text.single_mut() {
        set(&mut text, line);
    }

    // Terrorist hunt status.
    if let Ok(mut text) = hunt_text.single_mut() {
        let line = if rules.hunt && matches!(state.phase, Phase::Live) { format!("HOSTILES  {}      SQUAD LIVES  {}", state.hostiles, state.lives) } else { String::new() };
        set(&mut text, line);
    }
}
