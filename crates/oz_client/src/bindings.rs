//! Remappable keyboard and mouse controls.
//!
//! Every gameplay action reads its key through [`Bindings`], which lives in
//! the settings file as `bind.<action>=<key>` lines. The CONTROLS screen
//! (from SETTINGS) lists them; pick one and press the new key or mouse
//! button. A key already used elsewhere swaps places, so nothing is lost.
//! Weapon number keys, Escape and the menu keys stay fixed.

use crate::{
    ClientSet,
    input::Controls,
    settings::Settings,
    ui::{self, Act, ActEvent, Order, Screen, UiDirty},
};
use bevy::prelude::*;
use oz_sim::prelude::MatchPhase;

/// A physical key or mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bind {
    Key(KeyCode),
    Mouse(MouseButton),
}

impl Bind {
    pub fn pressed(self, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> bool {
        match self {
            Bind::Key(key) => keys.pressed(key),
            Bind::Mouse(button) => mouse.pressed(button),
        }
    }

    pub fn just_pressed(self, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> bool {
        match self {
            Bind::Key(key) => keys.just_pressed(key),
            Bind::Mouse(button) => mouse.just_pressed(button),
        }
    }

    /// Short name for prompts and the settings file.
    pub fn name(self) -> String {
        match self {
            Bind::Mouse(MouseButton::Left) => "MOUSE1".into(),
            Bind::Mouse(MouseButton::Right) => "MOUSE2".into(),
            Bind::Mouse(MouseButton::Middle) => "MOUSE3".into(),
            Bind::Mouse(MouseButton::Back) => "MOUSE4".into(),
            Bind::Mouse(MouseButton::Forward) => "MOUSE5".into(),
            Bind::Mouse(MouseButton::Other(n)) => format!("MOUSE{}", n as u32 + 6),
            Bind::Key(key) => KEYS.iter().find(|(code, _)| *code == key).map_or("?".into(), |(_, name)| (*name).into()),
        }
    }

    pub fn parse(text: &str) -> Option<Bind> {
        let text = text.trim();
        if let Some(n) = text.strip_prefix("MOUSE").and_then(|n| n.parse::<u16>().ok()) {
            return Some(Bind::Mouse(match n {
                1 => MouseButton::Left,
                2 => MouseButton::Right,
                3 => MouseButton::Middle,
                4 => MouseButton::Back,
                5 => MouseButton::Forward,
                n => MouseButton::Other(n.saturating_sub(6)),
            }));
        }
        KEYS.iter().find(|(_, name)| *name == text).map(|(code, _)| Bind::Key(*code))
    }
}

/// Keys that can be bound, with their names.
const KEYS: &[(KeyCode, &str)] = &[
    (KeyCode::KeyA, "A"),
    (KeyCode::KeyB, "B"),
    (KeyCode::KeyC, "C"),
    (KeyCode::KeyD, "D"),
    (KeyCode::KeyE, "E"),
    (KeyCode::KeyF, "F"),
    (KeyCode::KeyG, "G"),
    (KeyCode::KeyH, "H"),
    (KeyCode::KeyI, "I"),
    (KeyCode::KeyJ, "J"),
    (KeyCode::KeyK, "K"),
    (KeyCode::KeyL, "L"),
    (KeyCode::KeyM, "M"),
    (KeyCode::KeyN, "N"),
    (KeyCode::KeyO, "O"),
    (KeyCode::KeyP, "P"),
    (KeyCode::KeyQ, "Q"),
    (KeyCode::KeyR, "R"),
    (KeyCode::KeyS, "S"),
    (KeyCode::KeyT, "T"),
    (KeyCode::KeyU, "U"),
    (KeyCode::KeyV, "V"),
    (KeyCode::KeyW, "W"),
    (KeyCode::KeyX, "X"),
    (KeyCode::KeyY, "Y"),
    (KeyCode::KeyZ, "Z"),
    (KeyCode::Digit0, "0"),
    (KeyCode::Digit1, "1"),
    (KeyCode::Digit2, "2"),
    (KeyCode::Digit3, "3"),
    (KeyCode::Digit4, "4"),
    (KeyCode::Digit5, "5"),
    (KeyCode::Digit6, "6"),
    (KeyCode::Digit7, "7"),
    (KeyCode::Digit8, "8"),
    (KeyCode::Digit9, "9"),
    (KeyCode::F1, "F1"),
    (KeyCode::F2, "F2"),
    (KeyCode::F3, "F3"),
    (KeyCode::F4, "F4"),
    (KeyCode::F5, "F5"),
    (KeyCode::F6, "F6"),
    (KeyCode::F7, "F7"),
    (KeyCode::F8, "F8"),
    (KeyCode::F9, "F9"),
    (KeyCode::F10, "F10"),
    (KeyCode::F11, "F11"),
    (KeyCode::F12, "F12"),
    (KeyCode::Space, "SPACE"),
    (KeyCode::Tab, "TAB"),
    (KeyCode::CapsLock, "CAPS"),
    (KeyCode::Enter, "ENTER"),
    (KeyCode::Backspace, "BACKSPACE"),
    (KeyCode::ShiftLeft, "LSHIFT"),
    (KeyCode::ShiftRight, "RSHIFT"),
    (KeyCode::ControlLeft, "LCTRL"),
    (KeyCode::ControlRight, "RCTRL"),
    (KeyCode::AltLeft, "LALT"),
    (KeyCode::AltRight, "RALT"),
    (KeyCode::ArrowUp, "UP"),
    (KeyCode::ArrowDown, "DOWN"),
    (KeyCode::ArrowLeft, "LEFT"),
    (KeyCode::ArrowRight, "RIGHT"),
    (KeyCode::Backquote, "`"),
    (KeyCode::Minus, "-"),
    (KeyCode::Equal, "="),
    (KeyCode::BracketLeft, "["),
    (KeyCode::BracketRight, "]"),
    (KeyCode::Semicolon, ";"),
    (KeyCode::Quote, "'"),
    (KeyCode::Comma, ","),
    (KeyCode::Period, "."),
    (KeyCode::Slash, "/"),
    (KeyCode::Backslash, "\\"),
    (KeyCode::Insert, "INSERT"),
    (KeyCode::Delete, "DELETE"),
    (KeyCode::Home, "HOME"),
    (KeyCode::End, "END"),
    (KeyCode::PageUp, "PGUP"),
    (KeyCode::PageDown, "PGDN"),
    (KeyCode::Numpad0, "NUM0"),
    (KeyCode::Numpad1, "NUM1"),
    (KeyCode::Numpad2, "NUM2"),
    (KeyCode::Numpad3, "NUM3"),
    (KeyCode::Numpad4, "NUM4"),
    (KeyCode::Numpad5, "NUM5"),
    (KeyCode::Numpad6, "NUM6"),
    (KeyCode::Numpad7, "NUM7"),
    (KeyCode::Numpad8, "NUM8"),
    (KeyCode::Numpad9, "NUM9"),
];

/// Everything a player can rebind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Forward,
    Back,
    Left,
    Right,
    Fire,
    Aim,
    Reload,
    Interact,
    Jump,
    Sprint,
    Crouch,
    Prone,
    Cover,
    Gear,
    Inspect,
    Loadout,
    Scoreboard,
    Chat,
    Killstreak,
    NightVision,
    Melee,
}

impl Action {
    pub const ALL: [Action; 21] = [
        Action::Forward,
        Action::Back,
        Action::Left,
        Action::Right,
        Action::Fire,
        Action::Aim,
        Action::Reload,
        Action::Interact,
        Action::Jump,
        Action::Sprint,
        Action::Crouch,
        Action::Prone,
        Action::Cover,
        Action::Gear,
        Action::Inspect,
        Action::Loadout,
        Action::Scoreboard,
        Action::Chat,
        Action::Killstreak,
        Action::NightVision,
        Action::Melee,
    ];

    /// Shown on the controls screen.
    pub fn title(self) -> &'static str {
        match self {
            Action::Forward => "MOVE FORWARD",
            Action::Back => "MOVE BACK",
            Action::Left => "MOVE LEFT",
            Action::Right => "MOVE RIGHT",
            Action::Fire => "FIRE",
            Action::Aim => "AIM",
            Action::Reload => "RELOAD",
            Action::Interact => "USE",
            Action::Jump => "JUMP / VAULT",
            Action::Sprint => "SPRINT / STABILIZE",
            Action::Crouch => "CROUCH (HOLD: PRONE)",
            Action::Prone => "PRONE",
            Action::Melee => "MELEE",
            Action::Cover => "TAKE COVER",
            Action::Killstreak => "KILLSTREAK",
            Action::NightVision => "NIGHT VISION",
            Action::Gear => "THROW GEAR",
            Action::Inspect => "INSPECT WEAPON",
            Action::Loadout => "CHANGE CLASS",
            Action::Scoreboard => "SCOREBOARD",
            Action::Chat => "CHAT",
        }
    }

    /// Key in the settings file.
    fn id(self) -> &'static str {
        match self {
            Action::Forward => "forward",
            Action::Back => "back",
            Action::Left => "left",
            Action::Right => "right",
            Action::Fire => "fire",
            Action::Aim => "aim",
            Action::Reload => "reload",
            Action::Interact => "use",
            Action::Jump => "jump",
            Action::Sprint => "sprint",
            Action::Crouch => "crouch",
            Action::Prone => "prone",
            Action::Melee => "melee",
            Action::Cover => "cover",
            Action::Killstreak => "killstreak",
            Action::NightVision => "night_vision",
            Action::Gear => "gear",
            Action::Inspect => "inspect",
            Action::Loadout => "loadout",
            Action::Scoreboard => "scoreboard",
            Action::Chat => "chat",
        }
    }

    fn default_bind(self) -> Bind {
        use KeyCode as K;
        match self {
            Action::Forward => Bind::Key(K::KeyW),
            Action::Back => Bind::Key(K::KeyS),
            Action::Left => Bind::Key(K::KeyA),
            Action::Right => Bind::Key(K::KeyD),
            Action::Fire => Bind::Mouse(MouseButton::Left),
            Action::Aim => Bind::Mouse(MouseButton::Right),
            Action::Reload => Bind::Key(K::KeyR),
            Action::Interact => Bind::Key(K::KeyF),
            Action::Jump => Bind::Key(K::Space),
            Action::Sprint => Bind::Key(K::ShiftLeft),
            Action::Crouch => Bind::Key(K::KeyC),
            Action::Prone => Bind::Key(K::KeyZ),
            Action::Cover => Bind::Key(K::KeyQ),
            Action::Killstreak => Bind::Key(K::KeyX),
            Action::NightVision => Bind::Key(K::KeyN),
            Action::Gear => Bind::Key(K::KeyG),
            Action::Inspect => Bind::Key(K::KeyI),
            Action::Loadout => Bind::Key(K::KeyL),
            Action::Scoreboard => Bind::Key(K::Tab),
            Action::Chat => Bind::Key(K::KeyT),
            Action::Melee => Bind::Key(K::KeyE),
        }
    }
}

/// The current key for every [`Action`], in [`Action::ALL`] order.
#[derive(Clone, Debug, PartialEq)]
pub struct Bindings(pub [Bind; Action::ALL.len()]);

impl Default for Bindings {
    fn default() -> Self {
        Self(Action::ALL.map(Action::default_bind))
    }
}

impl Bindings {
    pub fn get(&self, action: Action) -> Bind {
        self.0[action as usize]
    }

    /// Short name of an action's key, for on-screen prompts.
    pub fn label(&self, action: Action) -> String {
        self.get(action).name()
    }

    /// Binds `action` to `bind`; whatever held that key takes the old one.
    pub fn set(&mut self, action: Action, bind: Bind) {
        let old = self.0[action as usize];
        for slot in self.0.iter_mut() {
            if *slot == bind {
                *slot = old;
            }
        }
        self.0[action as usize] = bind;
    }

    /// Reads a `bind.<id>` line from the settings file.
    pub fn load_line(&mut self, key: &str, value: &str) {
        let Some(id) = key.strip_prefix("bind.") else {
            return;
        };
        if let (Some(action), Some(bind)) = (Action::ALL.iter().find(|a| a.id() == id), Bind::parse(value)) {
            self.0[*action as usize] = bind;
        }
    }

    pub fn save_lines(&self) -> String {
        Action::ALL.iter().map(|a| format!("bind.{}={}\n", a.id(), self.get(*a).name())).collect()
    }
}

/// The action waiting for a new key, if any.
#[derive(Resource, Default)]
pub struct Capture {
    pub action: Option<Action>,
    /// Becomes true once every button from the click that started the capture is released.
    armed: bool,
}

pub struct BindingsPlugin;

impl Plugin for BindingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Capture>().add_systems(Update, (capture, binding_actions).chain().in_set(ClientSet::Input).after(crate::input::ReadDevices).before(crate::menu::MenuActions));
    }
}

fn capture(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut capture: ResMut<Capture>,
    mut settings: ResMut<Settings>,
    mut controls: ResMut<Controls>,
    mut dirty: ResMut<UiDirty>,
    screen: Res<Screen>,
) {
    let Some(action) = capture.action else { return };
    if *screen != Screen::Controls {
        capture.action = None;
        return;
    }
    // Nothing else in the menus reacts while a key is being chosen.
    controls.ui = default();
    if !capture.armed {
        capture.armed = keys.get_pressed().next().is_none() && mouse.get_pressed().next().is_none();
        return;
    }
    let pressed = keys.get_just_pressed().next().map(|k| Bind::Key(*k)).or_else(|| mouse.get_just_pressed().next().map(|m| Bind::Mouse(*m)));
    let Some(bind) = pressed else { return };
    capture.action = None;
    dirty.0 = true;
    if bind == Bind::Key(KeyCode::Escape) {
        return;
    }
    if bind.name() != "?" {
        settings.bindings.set(action, bind);
        settings.save();
    }
}

fn binding_actions(mut events: EventReader<ActEvent>, mut capture: ResMut<Capture>, mut settings: ResMut<Settings>, mut dirty: ResMut<UiDirty>) {
    for ActEvent(act) in events.read() {
        match act {
            Act::Rebind(index) => {
                if let Some(action) = Action::ALL.get(*index as usize) {
                    *capture = Capture { action: Some(*action), armed: false };
                    dirty.0 = true;
                }
            }
            Act::ResetBindings => {
                settings.bindings = Bindings::default();
                settings.save();
                dirty.0 = true;
            }
            _ => {}
        }
    }
}

/// The CONTROLS screen: two columns of actions and their keys.
pub fn build_controls(mut commands: Commands, dirty: Res<UiDirty>, screen: Res<Screen>, settings: Res<Settings>, capture: Res<Capture>, phase: Res<State<MatchPhase>>) {
    if *screen != Screen::Controls || !dirty.0 {
        return;
    }
    let mut order = Order::default();
    let order = &mut order;
    let in_match = *phase.get() == MatchPhase::Active;
    let mut root = if in_match { ui::root(&mut commands, 0.75) } else { ui::root_left(&mut commands, 0.0) };
    root.with_children(|root| {
        ui::label(root, "CONTROLS", 34.0, ui::TEXT);
        root.spawn(ui::panel(700.0)).with_children(|panel| {
            panel.spawn(ui::row(8.0)).with_children(|grid| {
                for (index, action) in Action::ALL.iter().enumerate() {
                    let key = if capture.action == Some(*action) { "PRESS KEY".to_string() } else { settings.bindings.label(*action) };
                    ui::sized_button(grid, order, format!("{:<20} {key}", action.title()), Act::Rebind(index as u8), None, Val::Px(320.0));
                }
            });
            let hint = if capture.action.is_some() { "PRESS ANY KEY OR MOUSE BUTTON.  ESC CANCELS." } else { "CLICK AN ACTION, THEN PRESS ITS NEW KEY. A KEY IN USE SWAPS PLACES." };
            ui::label(panel, hint, 13.0, if capture.action.is_some() { ui::GOOD } else { ui::DIM });
            ui::label(panel, "1-9 / SCROLL WEAPONS    ESC PAUSE    GAMEPAD: SEE README", 13.0, ui::DIM);
            panel.spawn(ui::row(8.0)).with_children(|row| {
                ui::sized_button(row, order, "RESET TO DEFAULTS", Act::ResetBindings, None, Val::Px(320.0));
                ui::sized_button(row, order, "BACK", Act::Back, None, Val::Px(320.0));
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_rebinding_swaps() {
        for (code, _) in KEYS {
            assert_eq!(Bind::parse(&Bind::Key(*code).name()), Some(Bind::Key(*code)));
        }
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle, MouseButton::Back, MouseButton::Forward, MouseButton::Other(3)] {
            assert_eq!(Bind::parse(&Bind::Mouse(button).name()), Some(Bind::Mouse(button)));
        }
        let mut binds = Bindings::default();
        binds.set(Action::Jump, Bind::Key(KeyCode::KeyC));
        assert_eq!(binds.get(Action::Jump), Bind::Key(KeyCode::KeyC));
        assert_eq!(binds.get(Action::Crouch), Bind::Key(KeyCode::Space));
        let mut loaded = Bindings::default();
        for line in binds.save_lines().lines() {
            let (key, value) = line.split_once('=').unwrap();
            loaded.load_line(key, value);
        }
        assert_eq!(loaded, binds);
    }
}
