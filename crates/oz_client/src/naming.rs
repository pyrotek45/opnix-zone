//! Choosing your name.
//!
//! EDIT NAME (main menu, settings, join screen and the lobby) opens a small editor: type
//! on the keyboard, or walk an on-screen keyboard with the d-pad and press (A) on each
//! letter. DONE keeps it: the name is saved as the callsign and, in a match, sent to the
//! host, which cleans it up, makes it unique and shows it to everyone.

use crate::{
    ClientSet,
    input::Controls,
    settings::Settings,
    ui::{self, Act, ActEvent, FocusRequest, Order, Screen, UiDirty},
};
use bevy::{
    input::keyboard::{Key, KeyboardInput},
    prelude::*,
};
use oz_sim::{
    prelude::*,
    versus::{NAME_MAX, clean_name},
};

/// The name being written and the screen to go back to.
#[derive(Resource, Default)]
pub struct NameEdit {
    pub draft: String,
    back: Option<Screen>,
}

const KEYS: [&str; 4] = ["ABCDEFGHIJ", "KLMNOPQRST", "UVWXYZ-.!?", "0123456789"];

pub struct NamingPlugin;

impl Plugin for NamingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NameEdit>()
            .add_systems(Update, type_name.in_set(ClientSet::Input).after(crate::input::ReadDevices).before(ui::UiInteract))
            // After every other screen's handlers, so the Back that closes this screen is not
            // also taken by the screen it returns to.
            .add_systems(Update, name_actions.in_set(ClientSet::Input).after(crate::menu::MenuActions));
    }
}

/// Adds one typed character, keeping to what a name may hold.
fn push(draft: &mut String, c: char) {
    let c = c.to_ascii_uppercase();
    if draft.len() >= NAME_MAX {
        return;
    }
    if c == ' ' {
        if !draft.is_empty() && !draft.ends_with(' ') {
            draft.push(' ');
        }
    } else if clean_name(&c.to_string()).len() == 1 {
        draft.push(c);
    }
}

/// Keyboard typing on the name screen. Enter keeps the name (and is taken away from the
/// on-screen keyboard, which would otherwise also type the letter under the cursor).
fn type_name(mut keys: EventReader<KeyboardInput>, screen: Res<Screen>, mut edit: ResMut<NameEdit>, mut controls: ResMut<Controls>, mut acts: EventWriter<ActEvent>, mut dirty: ResMut<UiDirty>) {
    if *screen != Screen::Name {
        keys.clear();
        return;
    }
    for event in keys.read().filter(|event| event.state.is_pressed()) {
        match &event.logical_key {
            Key::Character(typed) => typed.chars().for_each(|c| push(&mut edit.draft, c)),
            Key::Space => push(&mut edit.draft, ' '),
            Key::Backspace => {
                edit.draft.pop();
            }
            Key::Enter => {
                controls.ui.accept = false;
                acts.write(ActEvent(Act::NameDone));
                continue;
            }
            _ => continue,
        }
        dirty.0 = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn name_actions(
    mut events: EventReader<ActEvent>,
    mut screen: ResMut<Screen>,
    mut edit: ResMut<NameEdit>,
    mut settings: ResMut<Settings>,
    mut dirty: ResMut<UiDirty>,
    players: Query<&Player, With<LocalPlayer>>,
    mut orders: EventWriter<CommandEvent>,
    mut request: ResMut<FocusRequest>,
) {
    for ActEvent(act) in events.read() {
        match act {
            Act::EditName if *screen != Screen::Name => {
                edit.draft = clean_name(&settings.callsign);
                edit.back = Some(screen.clone());
                *screen = Screen::Name;
            }
            Act::NameKey(c) if *screen == Screen::Name => push(&mut edit.draft, *c),
            Act::NameDel if *screen == Screen::Name => {
                edit.draft.pop();
            }
            Act::NameDone if *screen == Screen::Name => {
                let name = clean_name(&edit.draft);
                if !name.is_empty() {
                    settings.callsign = name.clone();
                    settings.save();
                    // In a match: the host renames us for everyone.
                    if let Ok(player) = players.single() {
                        orders.write(CommandEvent { player: player.id, command: PlayerCommand::Rename(name) });
                    }
                }
                *screen = edit.back.take().unwrap_or(Screen::Main);
                request.0 = Some(Act::EditName);
            }
            Act::Back if *screen == Screen::Name => {
                *screen = edit.back.take().unwrap_or(Screen::Main);
                request.0 = Some(Act::EditName);
            }
            _ => continue,
        }
        dirty.0 = true;
    }
}

pub fn build_name(mut commands: Commands, dirty: Res<UiDirty>, screen: Res<Screen>, edit: Res<NameEdit>, controls: Res<Controls>) {
    if !dirty.0 || *screen != Screen::Name {
        return;
    }
    let mut order = Order::default();
    let order = &mut order;
    ui::root(&mut commands, 0.7).with_children(|root| {
        ui::label(root, "YOUR NAME", 34.0, ui::TEXT);
        root.spawn(ui::panel(660.0)).with_children(|panel| {
            panel.spawn((Node { padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)), border: UiRect::all(Val::Px(2.0)), ..default() }, BorderColor(ui::ACCENT), BackgroundColor(Color::srgba(0.0, 0.02, 0.04, 0.9)))).with_children(|field| {
                let cursor = if edit.draft.len() < NAME_MAX { "_" } else { "" };
                ui::label(field, format!("{}{cursor}", edit.draft), 30.0, ui::GOOD);
            });
            ui::label(panel, format!("{} / {NAME_MAX} CHARACTERS", edit.draft.len()), 13.0, ui::DIM);
            for (row, keys) in KEYS.iter().enumerate() {
                panel.spawn(Node { column_gap: Val::Px(6.0), ..default() }).with_children(|line| {
                    for (col, key) in keys.chars().enumerate() {
                        ui::grid_button(line, order, key.to_string(), Act::NameKey(key), None, Val::Px(54.0), col as i32, row as i32);
                    }
                });
            }
            panel.spawn(Node { column_gap: Val::Px(6.0), ..default() }).with_children(|line| {
                let row = KEYS.len() as i32;
                ui::grid_button(line, order, "SPACE", Act::NameKey(' '), None, Val::Px(146.0), 0, row);
                ui::grid_button(line, order, "DELETE", Act::NameDel, None, Val::Px(146.0), 3, row);
                ui::grid_button(line, order, "DONE", Act::NameDone, None, Val::Px(146.0), 6, row);
                ui::grid_button(line, order, "CANCEL", Act::Back, None, Val::Px(146.0), 9, row);
            });
            let hint = if controls.gamepad { "D-PAD PICKS A KEY   (A) TYPES IT   DONE KEEPS THE NAME   (B) CANCELS" } else { "TYPE YOUR NAME   BACKSPACE DELETES   ENTER KEEPS IT   ESC CANCELS" };
            ui::label(panel, hint, 14.0, ui::DIM);
        });
    });
}
