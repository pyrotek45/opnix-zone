//! Device input. Keyboard, mouse and gamepad all feed one [`Controls`]
//! resource; nothing else in the client reads devices directly, so rebinding
//! or adding a device only ever touches this file.
//!
//! | Action          | Keyboard / mouse        | Gamepad                     |
//! |-----------------|-------------------------|-----------------------------|
//! | Move / look     | WASD / mouse            | left / right stick          |
//! | Fire / aim      | LMB / RMB               | RT / LT                     |
//! | Reload / use    | R / F                   | X                           |
//! | Jump / sprint   | Space / Shift           | A / click left stick        |
//! | Crouch / prone  | C or Ctrl / Z           | B tap / B hold              |
//! | Hold breath     | Shift while scoped      | click left stick while scoped |
//! | Weapons         | 1-9, wheel              | Y                           |
//! | Take cover      | Q (near a wall)         | LB                          |
//! | Gear            | G                       | RB                          |
//! | Inspect weapon  | I                       | D-pad up                    |
//! | Class / scores  | L / hold Tab            | D-pad down / hold Back      |
//! | Chat / spectate | T / V                   | -                           |
//! | Pause / back    | Esc                     | Start / B                   |
//! | Menus           | arrows, Enter, mouse    | D-pad or stick, A, LB / RB for tabs |
//!
//! Keyboard and mouse keys can be changed in SETTINGS > CONTROLS (see `bindings`).

use crate::{
    bindings::Action,ClientSet, settings::Settings, ui::Screen};
use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    prelude::*,
    window::{CursorGrabMode, PrimaryWindow},
};
use oz_core::arsenal::resolve;
use oz_sim::prelude::*;

/// Menu navigation, valid for exactly one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct UiNav {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub accept: bool,
    pub back: bool,
    pub tab_prev: bool,
    pub tab_next: bool,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct Controls {
    pub movement: Vec2,
    /// Yaw / pitch change for this frame, in radians.
    pub look: Vec2,
    pub fire: bool,
    pub fire_pressed: bool,
    pub aim: bool,
    pub aim_pressed: bool,
    pub reload: bool,
    pub reload_pressed: bool,
    pub jump: bool,
    pub sprint: bool,
    pub interact: bool,
    /// Toggle crouch this frame.
    pub crouch: bool,
    /// Toggle prone this frame.
    pub prone: bool,
    /// Throw / use equipment (held).
    pub gear: bool,
    pub inspect: bool,
    /// Open the loadout screen (versus modes).
    pub loadout: bool,
    /// Show the scoreboard (held).
    pub scoreboard: bool,
    /// Open text chat.
    pub chat: bool,
    /// Take or leave cover this frame.
    pub cover: bool,
    /// Call in a killstreak reward this frame.
    pub killstreak: bool,
    pub weapon_step: i32,
    pub weapon_slot: Option<u8>,
    pub pause: bool,
    pub ui: UiNav,
    /// The most recently used device, so prompts can show the right button names.
    pub gamepad: bool,
}

/// Label for the system that fills [`Controls`]; UI systems run after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReadDevices;

/// Label for the system that turns controls into the local `PlayerInput`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct DrivePlayer;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Controls>()
            .add_systems(Update, (read_devices.in_set(ReadDevices), grab_cursor, drive_player.in_set(DrivePlayer).run_if(in_state(MatchPhase::Active))).chain().in_set(ClientSet::Input));
    }
}

fn stick(value: Vec2) -> Vec2 {
    let length = value.length();
    if length < 0.18 { Vec2::ZERO } else { value / length * ((length - 0.18) / 0.82).min(1.0) }
}

#[allow(clippy::too_many_arguments)]
fn read_devices(
    time: Res<Time<Real>>,
    settings: Res<Settings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    gamepads: Query<&Gamepad>,
    mut controls: ResMut<Controls>,
    mut stick_nav: Local<[bool; 4]>,
    mut crouch_hold: Local<f32>,
) {
    let was_gamepad = controls.gamepad;
    let mut c = Controls { gamepad: was_gamepad, ..default() };
    let tap = |code| keys.just_pressed(code);

    // Keyboard and mouse, through the player's bindings.
    let binds = &settings.bindings;
    let down = |action| binds.get(action).pressed(&keys, &mouse);
    let hit = |action| binds.get(action).just_pressed(&keys, &mouse);
    c.movement = Vec2::new((down(Action::Right) as i8 - down(Action::Left) as i8) as f32, (down(Action::Forward) as i8 - down(Action::Back) as i8) as f32);
    let ads_scale = if down(Action::Aim) { settings.ads_sensitivity } else { 1.0 };
    c.look = motion.delta * 0.0022 * settings.sensitivity * ads_scale;
    c.fire = down(Action::Fire);
    c.fire_pressed = hit(Action::Fire);
    c.aim = down(Action::Aim);
    c.aim_pressed = hit(Action::Aim);
    c.reload = down(Action::Reload);
    c.reload_pressed = hit(Action::Reload);
    c.jump = down(Action::Jump);
    c.sprint = down(Action::Sprint);
    c.interact = hit(Action::Interact);
    c.crouch = hit(Action::Crouch);
    c.prone = hit(Action::Prone);
    c.gear = down(Action::Gear);
    c.inspect = hit(Action::Inspect);
    c.loadout = hit(Action::Loadout);
    c.scoreboard = down(Action::Scoreboard);
    c.chat = hit(Action::Chat);
    c.cover = hit(Action::Cover);
    c.killstreak = hit(Action::Killstreak);
    c.pause = tap(KeyCode::Escape);
    c.weapon_step = if scroll.delta.y > 0.0 { -1 } else if scroll.delta.y < 0.0 { 1 } else { 0 };
    let digits = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9];
    c.weapon_slot = digits.iter().position(|code| tap(*code)).map(|i| i as u8);
    c.ui = UiNav {
        up: tap(KeyCode::ArrowUp),
        down: tap(KeyCode::ArrowDown),
        left: tap(KeyCode::ArrowLeft),
        right: tap(KeyCode::ArrowRight),
        accept: tap(KeyCode::Enter) || tap(KeyCode::NumpadEnter),
        back: tap(KeyCode::Escape),
        tab_prev: tap(KeyCode::PageUp),
        tab_next: tap(KeyCode::PageDown) || tap(KeyCode::Tab),
    };
    if keys.get_just_pressed().next().is_some() || mouse.get_just_pressed().next().is_some() || motion.delta != Vec2::ZERO {
        c.gamepad = false;
    }

    // Gamepads: every connected pad contributes.
    for pad in &gamepads {
        let left = stick(pad.left_stick());
        let right = stick(pad.right_stick());
        let held = |button| pad.pressed(button);
        let hit = |button| pad.just_pressed(button);
        if left != Vec2::ZERO || right != Vec2::ZERO || pad.get_just_pressed().next().is_some() {
            c.gamepad = true;
        }
        if left.length() > c.movement.length() {
            c.movement = left;
        }
        let aiming = held(GamepadButton::LeftTrigger2);
        // Squared response for fine aim, slower while aiming down sights.
        let turn = right * right.length() * 3.4 * settings.sensitivity * if aiming { 0.45 * settings.ads_sensitivity } else { 1.0 } * time.delta_secs();
        c.look += Vec2::new(turn.x, -turn.y);
        c.fire |= held(GamepadButton::RightTrigger2);
        c.fire_pressed |= hit(GamepadButton::RightTrigger2);
        c.aim |= aiming;
        c.aim_pressed |= hit(GamepadButton::LeftTrigger2);
        c.reload |= held(GamepadButton::West);
        c.reload_pressed |= hit(GamepadButton::West);
        c.interact |= hit(GamepadButton::West);
        c.jump |= held(GamepadButton::South);
        c.sprint |= held(GamepadButton::LeftThumb);
        c.gear |= held(GamepadButton::RightTrigger);
        c.inspect |= hit(GamepadButton::DPadUp);
        c.cover |= hit(GamepadButton::LeftTrigger);
        c.killstreak |= hit(GamepadButton::RightThumb);
        c.loadout |= hit(GamepadButton::DPadDown);
        c.scoreboard |= held(GamepadButton::Select);
        c.pause |= hit(GamepadButton::Start);
        c.weapon_step += hit(GamepadButton::North) as i32;
        // B: tap to crouch, hold to go prone.
        if held(GamepadButton::East) {
            *crouch_hold += time.delta_secs();
            if *crouch_hold >= 0.35 && *crouch_hold - time.delta_secs() < 0.35 {
                c.prone = true;
            }
        } else {
            if *crouch_hold > 0.0 && *crouch_hold < 0.35 {
                c.crouch = true;
            }
            *crouch_hold = 0.0;
        }

        // The left stick doubles as a D-pad in menus (edge-triggered).
        let now = [left.y > 0.6, left.y < -0.6, left.x < -0.6, left.x > 0.6];
        let edge = |i: usize| now[i] && !stick_nav[i];
        c.ui.up |= hit(GamepadButton::DPadUp) || edge(0);
        c.ui.down |= hit(GamepadButton::DPadDown) || edge(1);
        c.ui.left |= hit(GamepadButton::DPadLeft) || edge(2);
        c.ui.right |= hit(GamepadButton::DPadRight) || edge(3);
        *stick_nav = now;
        c.ui.accept |= hit(GamepadButton::South);
        c.ui.back |= hit(GamepadButton::East) || hit(GamepadButton::Start);
        c.ui.tab_prev |= hit(GamepadButton::LeftTrigger);
        c.ui.tab_next |= hit(GamepadButton::RightTrigger);
    }
    c.movement = c.movement.clamp_length_max(1.0);
    if settings.invert_y {
        c.look.y = -c.look.y;
    }
    *controls = c;
}

/// The mouse is captured only while actually playing.
fn grab_cursor(screen: Res<Screen>, phase: Res<State<MatchPhase>>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let Ok(mut window) = windows.single_mut() else { return };
    let playing = *phase.get() == MatchPhase::Active && *screen == Screen::None && window.focused;
    let wanted = if playing { CursorGrabMode::Locked } else { CursorGrabMode::None };
    if window.cursor_options.grab_mode != wanted {
        window.cursor_options.grab_mode = wanted;
        window.cursor_options.visible = !playing;
    }
}

/// Client-side state behind [`drive_player`]: stance toggles and scope sway.
#[derive(Default)]
struct Body {
    stance: u8,
    hold_fire: bool,
    sway_time: f32,
    sway: Vec2,
    breath: f32,
}

/// Turns [`Controls`] into the local player's [`PlayerInput`].
#[allow(clippy::too_many_arguments)]
fn drive_player(
    time: Res<Time>,
    controls: Res<Controls>,
    screen: Res<Screen>,
    content: Res<Content>,
    placing: Res<crate::stations::Placement>,
    mut players: Query<(&mut PlayerInput, &Arsenal, &Health), With<LocalPlayer>>,
    mut body: Local<Body>,
) {
    let Ok((mut input, arsenal, health)) = players.single_mut() else { return };
    if *screen != Screen::None || !health.alive() {
        // A menu is open (or we are down): stand still, keep looking where we were.
        input.movement = Vec2::ZERO;
        input.buttons = 0;
        // The click that closes a menu must not also fire the gun.
        body.hold_fire = true;
        if !health.alive() {
            body.stance = stance::STAND;
            input.stance = stance::STAND;
        }
        return;
    }
    if !controls.fire {
        body.hold_fire = false;
    }

    // Stance: toggles, and anything athletic stands you up.
    if controls.crouch {
        body.stance = if body.stance == stance::CROUCH { stance::STAND } else { stance::CROUCH };
    }
    if controls.prone {
        body.stance = if body.stance == stance::PRONE { stance::STAND } else { stance::PRONE };
    }
    if controls.jump && body.stance != stance::STAND {
        body.stance = if body.stance == stance::PRONE { stance::CROUCH } else { stance::STAND };
    } else if controls.sprint && controls.movement.y > 0.3 && !controls.aim {
        body.stance = stance::STAND;
    }
    input.stance = body.stance;

    // Leaning out is cover's job (see `cover`).
    input.lean = 0;

    input.movement = controls.movement;
    let mut look = controls.look;

    // Magnified optics sway; going low steadies them, and holding your breath nearly stops it.
    let stats = content.weapons.get(arsenal.current as usize).zip(arsenal.state()).map(|(def, state)| resolve(def, state, &content.attachments));
    let scoped = controls.aim && stats.is_some_and(|s| s.scope > 0) && arsenal.reload <= 0.0;
    let dt = time.delta_secs();
    let mut holding = false;
    if scoped {
        holding = controls.sprint && body.breath < 4.0;
        body.breath = if holding { body.breath + dt } else { (body.breath - dt * 0.8).max(0.0) };
        let amplitude = [0.012, 0.006, 0.0025][body.stance.min(2) as usize] * if stats.is_some_and(|s| s.scope == 1) { 0.5 } else { 1.0 } * if holding { 0.12 } else { 1.0 };
        body.sway_time += dt;
        let t = body.sway_time;
        let sway = Vec2::new((t * 0.8).sin() + 0.4 * (t * 2.1).sin(), 0.5 * (t * 1.6).sin() + 0.3 * (t * 0.7).cos()) * amplitude;
        look += sway - body.sway;
        body.sway = sway;
    } else {
        body.sway = Vec2::ZERO;
        body.breath = (body.breath - dt).max(0.0);
    }
    input.yaw = (input.yaw - look.x).rem_euclid(std::f32::consts::TAU);
    input.pitch = (input.pitch - look.y).clamp(-1.45, 1.45);

    let building = placing.kind.is_some();
    let mut held = 0;
    for (on, bit) in [
        (controls.fire && !building && !body.hold_fire, buttons::FIRE),
        (controls.aim && !building, buttons::AIM),
        (controls.reload && !building, buttons::RELOAD),
        (controls.jump, buttons::JUMP),
        // Shift steadies the scope instead of sprinting while aiming.
        (controls.sprint && !holding, buttons::SPRINT),
        (controls.gear && !building, buttons::GEAR),
    ] {
        if on {
            held |= bit;
        }
    }
    input.buttons = held;

    // Weapon selection only ever lands on owned weapons.
    let owned: Vec<u8> = (0..arsenal.weapons.len() as u8).filter(|i| arsenal.owns(*i as usize)).collect();
    if let Some(slot) = controls.weapon_slot.and_then(|slot| owned.get(slot as usize)) {
        input.weapon = *slot;
    } else if controls.weapon_step != 0 && !owned.is_empty() {
        let at = owned.iter().position(|i| *i == input.weapon).unwrap_or(0) as i32;
        input.weapon = owned[(at + controls.weapon_step).rem_euclid(owned.len() as i32) as usize];
    }
}
