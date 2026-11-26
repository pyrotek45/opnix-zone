//! Device input. Keyboard, mouse and gamepad all feed one [`Controls`]
//! resource; nothing else in the client reads devices directly, so rebinding
//! or adding a device only ever touches this file.
//!
//! | Action        | Keyboard / mouse      | Gamepad              |
//! |---------------|-----------------------|----------------------|
//! | Move / look   | WASD / mouse          | left / right stick   |
//! | Fire / aim    | LMB / RMB             | RT / LT              |
//! | Reload        | R                     | X                    |
//! | Interact      | E                     | X (hold near a station: tap) |
//! | Jump / sprint | Space / Shift         | A / click left stick |
//! | Weapons       | 1-8, wheel, Q         | LB / RB, Y           |
//! | Pause / back  | Esc                   | Start / B            |
//! | Menus         | arrows, Enter, mouse  | D-pad or stick, A, B, LB / RB for tabs |

use crate::{ClientSet, settings::Settings, ui::Screen};
use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    prelude::*,
    window::{CursorGrabMode, PrimaryWindow},
};
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

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Controls>()
            .add_systems(Update, (read_devices.in_set(ReadDevices), grab_cursor, drive_player.run_if(in_state(MatchPhase::Active))).chain().in_set(ClientSet::Input));
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
) {
    let was_gamepad = controls.gamepad;
    let mut c = Controls { gamepad: was_gamepad, ..default() };
    let key = |code| keys.pressed(code);
    let tap = |code| keys.just_pressed(code);

    // Keyboard and mouse.
    c.movement = Vec2::new((key(KeyCode::KeyD) as i8 - key(KeyCode::KeyA) as i8) as f32, (key(KeyCode::KeyW) as i8 - key(KeyCode::KeyS) as i8) as f32);
    c.look = motion.delta * 0.0022 * settings.sensitivity;
    c.fire = mouse.pressed(MouseButton::Left);
    c.fire_pressed = mouse.just_pressed(MouseButton::Left);
    c.aim = mouse.pressed(MouseButton::Right);
    c.aim_pressed = mouse.just_pressed(MouseButton::Right);
    c.reload = key(KeyCode::KeyR);
    c.reload_pressed = tap(KeyCode::KeyR);
    c.jump = key(KeyCode::Space);
    c.sprint = key(KeyCode::ShiftLeft);
    c.interact = tap(KeyCode::KeyE);
    c.pause = tap(KeyCode::Escape);
    c.weapon_step = if scroll.delta.y > 0.0 { -1 } else if scroll.delta.y < 0.0 { 1 } else { 0 } + tap(KeyCode::KeyQ) as i32;
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
        let turn = right * right.length() * 3.4 * settings.sensitivity * if aiming { 0.45 } else { 1.0 } * time.delta_secs();
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
        c.pause |= hit(GamepadButton::Start);
        c.weapon_step += hit(GamepadButton::RightTrigger) as i32 + hit(GamepadButton::North) as i32 - hit(GamepadButton::LeftTrigger) as i32;

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

/// Turns [`Controls`] into the local player's [`PlayerInput`].
fn drive_player(
    controls: Res<Controls>,
    screen: Res<Screen>,
    placing: Res<crate::stations::Placement>,
    mut players: Query<(&mut PlayerInput, &Arsenal), With<LocalPlayer>>,
    mut hold_fire: Local<bool>,
) {
    let Ok((mut input, arsenal)) = players.single_mut() else { return };
    if *screen != Screen::None {
        // A menu is open: stand still, keep looking where we were.
        input.movement = Vec2::ZERO;
        input.buttons = 0;
        // The click that closes a menu must not also fire the gun.
        *hold_fire = true;
        return;
    }
    if !controls.fire {
        *hold_fire = false;
    }
    input.movement = controls.movement;
    input.yaw = (input.yaw - controls.look.x).rem_euclid(std::f32::consts::TAU);
    input.pitch = (input.pitch - controls.look.y).clamp(-1.45, 1.45);

    let building = placing.kind.is_some();
    let mut held = 0;
    for (on, bit) in [
        (controls.fire && !building && !*hold_fire, buttons::FIRE),
        (controls.aim && !building, buttons::AIM),
        (controls.reload && !building, buttons::RELOAD),
        (controls.jump, buttons::JUMP),
        (controls.sprint, buttons::SPRINT),
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
