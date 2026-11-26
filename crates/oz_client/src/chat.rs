//! Text chat: press T, type, Enter to send, Esc to cancel. Messages travel as
//! commands to the host, which relays them to everyone; the last few lines
//! show in the lower left and fade after a while.

use crate::{ClientSet, input::Controls, settings::sanitize, ui::Screen};
use bevy::{
    input::keyboard::{Key, KeyboardInput},
    prelude::*,
};
use oz_sim::prelude::*;

const SHOW_FOR: f32 = 10.0;
const LINES: usize = 6;

#[derive(Resource, Default)]
pub struct ChatLog {
    lines: Vec<(String, String, f32)>,
    draft: String,
}

#[derive(Component)]
struct ChatText;

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChatLog>()
            .add_systems(OnEnter(MatchPhase::Active), spawn_chat)
            .add_systems(Update, (open, type_line).chain().in_set(ClientSet::Input).after(crate::input::ReadDevices).before(crate::menu::MenuActions).run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, (collect, show).chain().in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

fn spawn_chat(mut commands: Commands, mut log: ResMut<ChatLog>) {
    *log = ChatLog::default();
    commands.spawn((
        MatchEntity,
        ChatText,
        Text::new(""),
        TextFont { font_size: 15.0, ..default() },
        TextColor(Color::srgb(0.85, 0.95, 0.9)),
        Node { position_type: PositionType::Absolute, left: Val::Px(22.0), bottom: Val::Px(110.0), max_width: Val::Px(520.0), ..default() },
        GlobalZIndex(-30),
    ));
}

fn open(controls: Res<Controls>, mut screen: ResMut<Screen>, mut log: ResMut<ChatLog>) {
    if controls.chat && *screen == Screen::None {
        *screen = Screen::Chat;
        log.draft.clear();
    }
}

fn type_line(mut keys: EventReader<KeyboardInput>, mut screen: ResMut<Screen>, mut log: ResMut<ChatLog>, players: Query<&Player, With<LocalPlayer>>, mut orders: EventWriter<CommandEvent>, mut opened: Local<bool>) {
    if *screen != Screen::Chat {
        *opened = false;
        keys.clear();
        return;
    }
    // Skip the T that opened the box.
    if !*opened {
        *opened = true;
        keys.clear();
        return;
    }
    for event in keys.read().filter(|e| e.state.is_pressed()) {
        match &event.logical_key {
            Key::Character(typed) => log.draft = sanitize(&format!("{}{typed}", log.draft), 80),
            Key::Space => log.draft = sanitize(&format!("{} ", log.draft), 80),
            Key::Backspace => {
                log.draft.pop();
            }
            Key::Enter => {
                if let Ok(player) = players.single() {
                    if !log.draft.trim().is_empty() {
                        orders.write(CommandEvent { player: player.id, command: PlayerCommand::Chat(log.draft.clone()) });
                    }
                }
                log.draft.clear();
                *screen = Screen::None;
                return;
            }
            Key::Escape => {
                log.draft.clear();
                *screen = Screen::None;
                return;
            }
            _ => {}
        }
    }
}

fn collect(time: Res<Time<Real>>, mut events: EventReader<FxEvent>, mut log: ResMut<ChatLog>) {
    let now = time.elapsed_secs();
    for event in events.read() {
        if let FxEvent::Chat { name, text } = event {
            log.lines.push((name.clone(), text.clone(), now));
            if log.lines.len() > LINES {
                log.lines.remove(0);
            }
        }
    }
    log.lines.retain(|(.., at)| now - at < SHOW_FOR);
}

fn show(screen: Res<Screen>, log: Res<ChatLog>, mut text: Query<&mut Text, With<ChatText>>) {
    let Ok(mut text) = text.single_mut() else { return };
    let mut lines: Vec<String> = log.lines.iter().map(|(name, line, _)| format!("{name}: {line}")).collect();
    if *screen == Screen::Chat {
        lines.push(format!("SAY: {}_    (ENTER SEND, ESC CANCEL)", log.draft));
    }
    let wanted = lines.join("\n");
    if text.0 != wanted {
        text.0 = wanted;
    }
}
