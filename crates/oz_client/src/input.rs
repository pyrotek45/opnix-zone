//! Device input. Keyboard, mouse and gamepad all feed one [`Controls`]
//! resource; nothing else in the client reads devices directly, so rebinding
//! or adding a device only ever touches this file.
//!
//! | Action          | Keyboard / mouse        | Gamepad                     |
//! |-----------------|-------------------------|-----------------------------|
//! | Move / look     | WASD / mouse            | left / right stick          |
//! | Fire / aim      | LMB / RMB               | RT / LT                     |
//! | Reload / use    | R / F                   | X                           |
//! | Jump / vault    | Space                   | A                           |
//! | Sprint          | Shift                   | click left stick            |
//! | Crouch / prone  | C tap / C hold, Z       | B tap / B hold              |
//! | Stabilize scope | Shift while scoped      | click left stick while scoped |
//! | Melee           | E                       | click right stick           |
//! | Weapons         | 1-9, wheel              | Y                           |
//! | Take / leave cover | Q (near a wall)      | LB                          |
//! | Gear            | hold G to cook, release to throw, R puts the pin back | hold RB |
//! | Killstreak      | X                       | D-pad right                 |
//! | Inspect weapon  | I                       | D-pad up                    |
//! | Class / scores  | L / hold Tab            | D-pad down / hold Back      |
//! | Chat / spectate | T / V                   | -                           |
//! | Pause / back    | Esc                     | Start / B                   |
//! | Menus           | arrows, Enter, mouse    | D-pad or stick, A, LB / RB for tabs |
//!
//! Keyboard and mouse keys can be changed in SETTINGS > CONTROLS (see `bindings`).

use crate::{ClientSet, bindings::Action, settings::Settings, ui::Screen};
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
    pub jump_pressed: bool,
    pub sprint: bool,
    pub sprint_pressed: bool,
    /// Crouch button (C / pad B) held: tap and hold mean different things (see `drive_player`).
    pub crouch_held: bool,
    /// Gun-butt strike this frame.
    pub melee: bool,
    /// Take-cover button held.
    pub cover_held: bool,
    /// Screw the suppressor on or off (tap D-pad down / U).
    pub suppressor: bool,
    /// Switch the flashlight (hold D-pad down / H).
    pub flashlight: bool,
    /// Pad X this frame: vaults when there is something to vault, else reload / use as usual.
    pub vault: bool,
    pub interact: bool,
    /// Use held down (planting or defusing a bomb).
    pub interact_held: bool,
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
    /// Toggle night vision this frame.
    pub night_vision: bool,
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
            .init_resource::<Steady>()
            .add_systems(Update, (read_devices.in_set(ReadDevices), grab_cursor, drive_player.in_set(DrivePlayer).run_if(in_state(MatchPhase::Active))).chain().in_set(ClientSet::Input));
    }
}

fn stick_with(value: Vec2, deadzone: f32) -> Vec2 {
    let length = value.length();
    if length < deadzone { Vec2::ZERO } else { value / length * ((length - deadzone) / (1.0 - deadzone)).min(1.0) }
}

#[allow(dead_code)]
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
    mut dpad_down: Local<f32>,
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
    c.jump_pressed = hit(Action::Jump);
    c.sprint = down(Action::Sprint);
    c.sprint_pressed = hit(Action::Sprint);
    c.crouch_held = down(Action::Crouch);
    c.melee = hit(Action::Melee);
    c.cover_held = down(Action::Cover);
    c.suppressor = hit(Action::Suppressor);
    c.flashlight = hit(Action::Flashlight);
    c.interact = hit(Action::Interact);
    c.interact_held = down(Action::Interact);
    c.crouch = hit(Action::Crouch);
    c.prone = hit(Action::Prone);
    c.gear = down(Action::Gear);
    c.inspect = hit(Action::Inspect);
    c.loadout = hit(Action::Loadout);
    c.scoreboard = down(Action::Scoreboard);
    c.chat = hit(Action::Chat);
    c.cover = hit(Action::Cover);
    c.killstreak = hit(Action::Killstreak);
    c.night_vision = hit(Action::NightVision);
    c.pause = tap(KeyCode::Escape);
    c.weapon_step = if scroll.delta.y > 0.0 {
        -1
    } else if scroll.delta.y < 0.0 {
        1
    } else {
        0
    };
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
        // Movement starts at the lightest touch; the look stick keeps a small deadzone
        // against drift.
        let left = stick_with(pad.left_stick(), 0.06);
        let right = stick_with(pad.right_stick(), 0.1);
        let held = |button| pad.pressed(button);
        let hit = |button| pad.just_pressed(button);
        if left != Vec2::ZERO || right != Vec2::ZERO || pad.get_just_pressed().next().is_some() {
            c.gamepad = true;
        }
        if left.length() > c.movement.length() {
            c.movement = left;
        }
        let aiming = held(GamepadButton::LeftTrigger2);
        // Response: dynamic (finer than squared near the centre, the same full speed at the
        // edge, steeper on the way) or the plain squared ramp; slower aiming down sights.
        let r = right.length();
        // (The stick's own length already scales `right`, so standard ends up squared.)
        let response = if settings.dynamic_curve { 0.6 * r + 0.4 * r * r * r } else { r };
        let turn = right * response * 3.4 * settings.sensitivity * if aiming { 0.45 * settings.ads_sensitivity } else { 1.0 } * time.delta_secs();
        c.look += Vec2::new(turn.x, -turn.y);
        c.fire |= held(GamepadButton::RightTrigger2);
        c.fire_pressed |= hit(GamepadButton::RightTrigger2);
        c.aim |= aiming;
        c.aim_pressed |= hit(GamepadButton::LeftTrigger2);
        c.reload |= held(GamepadButton::West);
        c.reload_pressed |= hit(GamepadButton::West);
        c.interact |= hit(GamepadButton::West);
        c.interact_held |= held(GamepadButton::West);
        c.jump |= held(GamepadButton::South);
        c.jump_pressed |= hit(GamepadButton::South);
        c.sprint |= held(GamepadButton::LeftThumb);
        c.sprint_pressed |= hit(GamepadButton::LeftThumb);
        c.gear |= held(GamepadButton::RightTrigger);
        c.inspect |= hit(GamepadButton::DPadUp);
        c.cover |= hit(GamepadButton::LeftTrigger);
        c.cover_held |= held(GamepadButton::LeftTrigger);
        c.melee |= hit(GamepadButton::RightThumb);
        c.killstreak |= hit(GamepadButton::DPadRight);
        c.night_vision |= hit(GamepadButton::DPadLeft);
        // D-pad down, Rainbow-style: tap to screw the suppressor on or off, hold for the flashlight.
        // (The class picker is on the pause menu.)
        if held(GamepadButton::DPadDown) {
            let was = *dpad_down;
            *dpad_down += time.delta_secs();
            if was < 0.4 && *dpad_down >= 0.4 {
                c.flashlight = true;
            }
        } else {
            if *dpad_down > 0.0 && *dpad_down < 0.4 {
                c.suppressor = true;
            }
            *dpad_down = 0.0;
        }
        c.scoreboard |= held(GamepadButton::Select);
        c.pause |= hit(GamepadButton::Start);
        c.weapon_step += hit(GamepadButton::North) as i32;
        // B: tap to crouch, hold to go prone (worked out in `drive_player`).
        c.crouch_held |= held(GamepadButton::East);

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
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    let playing = *phase.get() == MatchPhase::Active && *screen == Screen::None && window.focused;
    let wanted = if playing { CursorGrabMode::Locked } else { CursorGrabMode::None };
    if window.cursor_options.grab_mode != wanted {
        window.cursor_options.grab_mode = wanted;
        window.cursor_options.visible = !playing;
    }
}

/// Seconds the crouch button must be held to go prone (or, from prone, stand).
const HOLD_FOR_PRONE: f32 = 0.32;

/// Scope stabilizing, for the HUD: a robot locks its servos and the sway slows to a stop.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct Steady {
    /// Seconds of lock left right now (0 = not stabilizing).
    pub left: f32,
    /// How long this lock lasts in total.
    pub total: f32,
    /// Seconds until the servos can lock again.
    pub cooldown: f32,
    /// 0 = swaying freely, 1 = locked still.
    pub hold: f32,
}

impl Steady {
    /// How long a lock lasts: longer braced low.
    pub fn duration(stance_now: u8) -> f32 {
        [2.5, 4.0, 6.0][(stance_now as usize).min(2)]
    }
}

/// Client-side state behind [`drive_player`]: stance toggles and scope sway.
#[derive(Default)]
struct Body {
    stance: u8,
    hold_fire: bool,
    sway_time: f32,
    sway: Vec2,
    /// How long the crouch button has been down, and whether this press already acted.
    crouch_for: f32,
    crouch_done: bool,
    crouch_was: bool,
    crouch_from: u8,
    /// X vaulted: no reload from that same press.
    vault_press: bool,
    /// Recoil: shots seen so far, shots into this burst, time since the last one,
    /// kick still to be applied to the aim, and how far the muzzle has climbed.
    shots: Option<u32>,
    burst: u32,
    since_shot: f32,
    kick: Vec2,
    climbed: f32,
}

/// Where a gun's muzzle goes on shot `n` of a burst: always up, climbing a little harder as
/// the burst goes on, and sideways along a wander that belongs to that gun (learn it, pull
/// against it). In units of the gun's recoil.
pub fn recoil_pattern(weapon: u8, n: u32) -> Vec2 {
    let seed = weapon as f32 * 1.37 + 0.4;
    let k = n as f32;
    let up = 1.0 + 0.35 * (k.min(10.0) / 10.0);
    let side = 0.6 * ((k * 0.7 + seed).sin() + 0.5 * (k * 1.9 + seed * 2.3).sin()) * if n < 3 { 0.3 } else { 1.0 };
    Vec2::new(side, up)
}

/// Stance from the crouch button. Each press does exactly one thing however long it is held,
/// so nothing flickers or repeats:
///
/// | stance when pressed | tap        | hold        |
/// |---------------------|------------|-------------|
/// | standing            | crouch (at once) | prone |
/// | crouching           | stand      | prone       |
/// | prone               | crouch     | stand       |
///
/// The prone key toggles between prone and crouching.
fn stance_from_buttons(body: &mut Body, held: bool, prone_pressed: bool, dt: f32) {
    if held && !body.crouch_was {
        body.crouch_for = 0.0;
        body.crouch_done = false;
        body.crouch_from = body.stance;
        if body.stance == stance::STAND {
            body.stance = stance::CROUCH;
        }
    }
    if held && !body.crouch_done {
        body.crouch_for += dt;
        if body.crouch_for >= HOLD_FOR_PRONE {
            body.crouch_done = true;
            body.stance = if body.crouch_from == stance::PRONE { stance::STAND } else { stance::PRONE };
        }
    }
    if !held && body.crouch_was && !body.crouch_done {
        body.crouch_done = true;
        match body.crouch_from {
            stance::CROUCH => body.stance = stance::STAND,
            stance::PRONE => body.stance = stance::CROUCH,
            _ => {}
        }
    }
    body.crouch_was = held;
    if prone_pressed {
        body.stance = if body.stance == stance::PRONE { stance::CROUCH } else { stance::PRONE };
    }
}

/// Turns [`Controls`] into the local player's [`PlayerInput`].
#[allow(clippy::too_many_arguments)]
fn drive_player(
    time: Res<Time>,
    controls: Res<Controls>,
    screen: Res<Screen>,
    content: Res<Content>,
    placing: Res<crate::stations::Placement>,
    (map, statics): (Res<Map>, Res<Statics>),
    mut steady: ResMut<Steady>,
    cover: Res<crate::cover::Cover>,
    (role, prediction): (Res<NetRole>, Res<crate::predict::Prediction>),
    mut players: Query<(Entity, &Transform, &mut PlayerInput, &Arsenal, &Health, &Mover), With<LocalPlayer>>,
    mut body: Local<Body>,
    mut whose: Local<Option<Entity>>,
) {
    let Ok((entity, transform, mut input, arsenal, health, mover)) = players.single_mut() else {
        return;
    };
    // A new match (or a rejoin) is a new body: nothing carries over - not a crouch, not a held trigger.
    if *whose != Some(entity) {
        *whose = Some(entity);
        *body = Body { hold_fire: true, crouch_was: controls.crouch_held, ..default() };
        *steady = Steady::default();
        input.stance = stance::STAND;
        input.lean = 0;
        input.cover = 0;
    }
    if *screen != Screen::None || !health.alive() {
        // A menu is open (or we are down): stand still, keep looking where we were.
        input.movement = Vec2::ZERO;
        input.buttons = 0;
        input.interact = false;
        // The click that closes a menu must not also fire the gun.
        body.hold_fire = true;
        body.crouch_was = controls.crouch_held;
        if !health.alive() {
            body.stance = stance::STAND;
            input.stance = stance::STAND;
            *steady = Steady::default();
        }
        return;
    }
    if !controls.fire {
        body.hold_fire = false;
    }
    let dt = time.delta_secs();

    stance_from_buttons(&mut body, controls.crouch_held, controls.prone, dt);
    // Sprinting gets you up.
    if controls.sprint && controls.movement.y > 0.3 && !controls.aim {
        body.stance = stance::STAND;
    }
    // Jump from low: vault whatever is in front if there is something, else stand up.
    let mut jump = controls.jump && body.stance == stance::STAND;
    // X on a pad vaults too, when there is something low in front to go over.
    if controls.vault && cover.wall.is_some_and(|w| w.low) {
        // Behind low cover, X goes over it (`cover` steers the vault).
        body.vault_press = true;
    } else if controls.vault && mover.vault.is_none() && body.stance != stance::PRONE {
        let wish = Quat::from_rotation_y(input.yaw) * Vec3::new(controls.movement.x, 0.0, -controls.movement.y);
        let dir = if wish.length() > 0.2 { wish } else { Quat::from_rotation_y(input.yaw) * Vec3::NEG_Z };
        if oz_sim::player::vault_target(&map, &statics, transform.translation, dir).is_some() {
            jump = true;
            body.stance = stance::STAND;
            body.vault_press = true;
        }
    }
    if !controls.reload {
        body.vault_press = false;
    }
    if controls.jump_pressed && body.stance != stance::STAND && mover.vault.is_none() {
        let wish = Quat::from_rotation_y(input.yaw) * Vec3::new(controls.movement.x, 0.0, -controls.movement.y);
        let dir = if wish.length() > 0.2 { wish } else { Quat::from_rotation_y(input.yaw) * Vec3::NEG_Z };
        if body.stance == stance::CROUCH && oz_sim::player::vault_target(&map, &statics, transform.translation, dir).is_some() {
            jump = true;
        }
        body.stance = stance::STAND;
    }
    if mover.vault.is_some() {
        body.stance = stance::STAND;
    }
    input.stance = body.stance;

    // Leaning out is cover's job (see `cover`).
    input.lean = 0;

    input.movement = controls.movement;
    let mut look = controls.look;
    // Through magnified glass the view turns slower, so higher zoom stays steerable: past
    // a 3x the look scales with the square root of how far the optic narrows the view.
    if controls.aim {
        let zoom = content.weapons.get(arsenal.current as usize).zip(arsenal.state()).map_or(1.0, |(def, state)| {
            let fov = resolve(def, state, &content.attachments).ads_fov.clamp(1.0, 34.0);
            ((fov * 0.5).to_radians().tan() / 17f32.to_radians().tan()).sqrt()
        });
        look *= zoom;
    }

    // Recoil: every shot kicks the aim itself (this is where the next round goes), up and to
    // the side along the gun's own pattern. Heavier guns kick harder; crouching, going prone
    // and aiming brace it. Let go of the trigger and the muzzle settles part of the way back.
    let shots = if *role == NetRole::Client { prediction.total } else { arsenal.shots };
    let fired = body.shots.map_or(0, |seen| shots.saturating_sub(seen)).min(4);
    body.shots = Some(shots);
    let dt_kick = time.delta_secs();
    body.since_shot += dt_kick;
    if body.since_shot > 0.32 {
        body.burst = 0;
    }
    if fired > 0
        && let Some(stats) = content.weapons.get(arsenal.current as usize).zip(arsenal.state()).map(|(def, state)| resolve(def, state, &content.attachments))
    {
        let brace = stance::recoil(body.stance) * if controls.aim { 0.8 } else { 1.0 };
        for _ in 0..fired {
            body.burst += 1;
            let kick = recoil_pattern(arsenal.current, body.burst) * stats.recoil * 0.0045 * brace;
            body.kick += kick;
            body.climbed += kick.y;
        }
        body.since_shot = 0.0;
    }
    // The kick lands over a few hundredths of a second, so it feels like a shove, not a jump.
    let landing = body.kick * (1.0 - (-dt_kick * 28.0).exp());
    body.kick -= landing;
    look += Vec2::new(landing.x, -landing.y);
    if body.since_shot > 0.15 && body.climbed > 0.0 {
        let settle = body.climbed * (1.0 - (-dt_kick * 5.0).exp());
        body.climbed -= settle;
        look.y += settle * 0.55;
    }
    // Pulling down against the climb uses it up, so recovery never drags you below your aim.
    if look.y > 0.0 && controls.look.y > 0.0 {
        body.climbed = (body.climbed - controls.look.y).max(0.0);
    }

    // Magnified optics sway. Going low steadies them, and stabilizing (L3 / Shift) locks the
    // servos: the sway slows to a stop where it is - the crosshair never jumps - for a few
    // seconds (longer crouched, longest prone), then needs a moment before it can lock again.
    let stats = content.weapons.get(arsenal.current as usize).zip(arsenal.state()).map(|(def, state)| resolve(def, state, &content.attachments));
    let scoped = controls.aim && stats.is_some_and(|s| s.scope > 0) && arsenal.reload <= 0.0;
    steady.cooldown = (steady.cooldown - dt).max(0.0);
    if scoped {
        if controls.sprint_pressed && steady.left <= 0.0 && steady.cooldown <= 0.0 {
            steady.total = Steady::duration(body.stance);
            steady.left = steady.total;
        }
        if steady.left > 0.0 {
            steady.left -= dt;
            if steady.left <= 0.0 {
                steady.left = 0.0;
                steady.cooldown = 2.0;
            }
        }
        let wanted = if steady.left > 0.0 { 1.0 } else { 0.0 };
        steady.hold += (wanted - steady.hold) * (1.0 - (-dt * 6.0).exp());
        let amplitude = [0.012, 0.006, 0.0025][body.stance.min(2) as usize] * if stats.is_some_and(|s| s.scope == 1) { 0.5 } else { 1.0 };
        // Slowing the sway's clock (rather than shrinking it) is what keeps the aim from moving.
        body.sway_time += dt * (1.0 - steady.hold);
        let t = body.sway_time;
        let sway = Vec2::new((t * 0.8).sin() + 0.4 * (t * 2.1).sin(), 0.5 * (t * 1.6).sin() + 0.3 * (t * 0.7).cos()) * amplitude;
        look += sway - body.sway;
        body.sway = sway;
    } else {
        body.sway = Vec2::ZERO;
        if steady.left > 0.0 {
            steady.left = 0.0;
            steady.cooldown = 1.0;
        }
        steady.hold = 0.0;
    }
    let holding = scoped;
    input.yaw = (input.yaw - look.x).rem_euclid(std::f32::consts::TAU);
    input.pitch = (input.pitch - look.y).clamp(-1.45, 1.45);

    let building = placing.kind.is_some();
    let mut held = 0;
    for (on, bit) in [
        (controls.fire && !building && !body.hold_fire, buttons::FIRE),
        (controls.aim && !building, buttons::AIM),
        (controls.reload && !building && !body.vault_press, buttons::RELOAD),
        (jump, buttons::JUMP),
        // Shift steadies the scope instead of sprinting while aiming.
        (controls.sprint && !holding, buttons::SPRINT),
        (controls.gear && !building, buttons::GEAR),
        (controls.melee && !building, buttons::MELEE),
    ] {
        if on {
            held |= bit;
        }
    }
    input.buttons = held;
    input.interact = controls.interact_held;

    // Weapon selection only ever lands on owned weapons.
    let owned: Vec<u8> = (0..arsenal.weapons.len() as u8).filter(|i| arsenal.owns(*i as usize)).collect();
    if let Some(slot) = controls.weapon_slot.and_then(|slot| owned.get(slot as usize)) {
        input.weapon = *slot;
    } else if controls.weapon_step != 0 && !owned.is_empty() {
        let at = owned.iter().position(|i| *i == input.weapon).unwrap_or(0) as i32;
        input.weapon = owned[(at + controls.weapon_step).rem_euclid(owned.len() as i32) as usize];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(body: &mut Body, frames: usize, held: bool) {
        for _ in 0..frames {
            stance_from_buttons(body, held, false, 1.0 / 60.0);
        }
    }

    #[test]
    fn crouch_taps_and_holds_cycle_without_flicker() {
        let mut body = Body::default();
        // Tap from standing: crouch on the press, stays crouched.
        run(&mut body, 3, true);
        assert_eq!(body.stance, stance::CROUCH);
        run(&mut body, 3, false);
        assert_eq!(body.stance, stance::CROUCH);
        // Hold from a crouch: prone, and it stays prone however long the button stays down.
        run(&mut body, 120, true);
        assert_eq!(body.stance, stance::PRONE);
        run(&mut body, 3, false);
        assert_eq!(body.stance, stance::PRONE);
        // Tap from prone: up to a crouch; tap again: stand.
        run(&mut body, 3, true);
        run(&mut body, 3, false);
        assert_eq!(body.stance, stance::CROUCH);
        run(&mut body, 3, true);
        run(&mut body, 3, false);
        assert_eq!(body.stance, stance::STAND);
        // Hold from standing goes all the way down; holding from prone stands up.
        run(&mut body, 60, true);
        run(&mut body, 2, false);
        assert_eq!(body.stance, stance::PRONE);
        run(&mut body, 60, true);
        run(&mut body, 2, false);
        assert_eq!(body.stance, stance::STAND);
    }
}
