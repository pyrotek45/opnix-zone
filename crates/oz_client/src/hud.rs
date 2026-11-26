//! The in-game heads-up display. Text is only rewritten when it changes, so
//! an idle HUD costs nothing.

use crate::{ClientSet, input::Controls, stations::{Nearby, Placement}, ui::{self, Screen}, view::ViewState};
use bevy::prelude::*;
use oz_core::arsenal::resolve;
use oz_sim::prelude::*;

/// Messages shown mid-screen for a few seconds.
#[derive(Resource, Default)]
pub struct Notices(Vec<(String, f32)>);

impl Notices {
    pub fn push(&mut self, text: String) {
        self.0.retain(|(existing, _)| *existing != text);
        self.0.push((text, 5.0));
        if self.0.len() > 4 {
            self.0.remove(0);
        }
    }
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Status,
    Wave,
    Cross,
    Notice,
    Prompt,
    Hint,
}

#[derive(Component)]
struct HudRoot;

#[derive(Component)]
struct HurtOverlay;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Notices>()
            .add_systems(OnEnter(MatchPhase::Active), spawn_hud)
            .add_systems(Update, update_hud.in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

fn spawn_hud(mut commands: Commands, mut notices: ResMut<Notices>) {
    notices.0.clear();
    let text = |slot: Slot, size: f32, color: Color, node: Node| (slot, Text::new(""), TextFont { font_size: size, ..default() }, TextColor(color), node);
    let at = |node: Node| Node { position_type: PositionType::Absolute, ..node };
    commands.spawn((MatchEntity, HurtOverlay, at(Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }), BackgroundColor(Color::NONE), GlobalZIndex(-50)));
    commands
        .spawn((MatchEntity, HudRoot, at(Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }), GlobalZIndex(-40)))
        .with_children(|hud| {
            hud.spawn(text(Slot::Status, 19.0, ui::TEXT, at(Node { left: Val::Px(22.0), top: Val::Px(18.0), ..default() })));
            hud.spawn(text(Slot::Wave, 20.0, ui::TEXT, at(Node { right: Val::Px(22.0), top: Val::Px(18.0), ..default() })));
            hud.spawn(text(Slot::Hint, 13.0, ui::DIM, at(Node { left: Val::Px(22.0), bottom: Val::Px(16.0), ..default() })));
            let centered = |top: f32| at(Node { width: Val::Percent(100.0), top: Val::Percent(top), justify_content: JustifyContent::Center, ..default() });
            hud.spawn(centered(48.2)).with_children(|c| {
                c.spawn(text(Slot::Cross, 24.0, ui::TEXT, Node::default()));
            });
            hud.spawn(centered(22.0)).with_children(|c| {
                c.spawn((text(Slot::Notice, 21.0, ui::TEXT, Node::default()), TextLayout::new_with_justify(JustifyText::Center)));
            });
            hud.spawn(centered(62.0)).with_children(|c| {
                c.spawn((text(Slot::Prompt, 19.0, ui::ACCENT, Node::default()), TextLayout::new_with_justify(JustifyText::Center)));
            });
        });
}

fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

#[allow(clippy::too_many_arguments)]
fn update_hud(
    time: Res<Time<Real>>,
    state: Res<MatchState>,
    content: Res<Content>,
    link: Res<NetLink>,
    screen: Res<Screen>,
    controls: Res<Controls>,
    view: Res<ViewState>,
    nearby: Res<Nearby>,
    placing: Res<Placement>,
    mut notices: ResMut<Notices>,
    players: Query<(&Health, &Arsenal, &Player), With<LocalPlayer>>,
    mut texts: Query<(&Slot, &mut Text, &mut TextColor)>,
    mut root: Query<&mut Visibility, With<HudRoot>>,
    mut hurt: Query<&mut BackgroundColor, With<HurtOverlay>>,
) {
    let dt = time.delta_secs();
    notices.0.retain_mut(|(_, ttl)| {
        *ttl -= dt;
        *ttl > 0.0
    });
    if let Ok(mut visibility) = root.single_mut() {
        let wanted = if *screen == Screen::None { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    let Ok((health, arsenal, player)) = players.single() else { return };
    if let Ok(mut overlay) = hurt.single_mut() {
        let low = if health.alive() { (0.35 - health.fraction()).max(0.0) * 0.6 } else { 0.45 };
        overlay.0 = Color::srgba(0.8, 0.05, 0.05, (view.hurt * 0.35 + low).min(0.6));
    }

    let index = arsenal.current as usize;
    let weapon = content.weapons.get(index).zip(arsenal.weapons.get(index));
    let weapon_line = weapon.map_or(String::new(), |(def, owned)| {
        let stats = resolve(def, owned, &content.attachments);
        if def.mag == 0 {
            format!("{}  READY", def.name)
        } else {
            let reserve = if def.infinite_reserve { "INF".into() } else { owned.reserve.to_string() };
            let note = if arsenal.reload > 0.0 { "  RELOADING" } else if owned.ammo == 0 { "  EMPTY" } else if owned.ammo * 4 <= stats.mag { "  LOW" } else { "" };
            format!("{}  {:02}/{:02} | {}{}", def.name, owned.ammo, stats.mag, reserve, note)
        }
    });
    let beacon = if state.objective_max > 0.0 { format!("BEACON {:.0}/{:.0}\n", state.objective_hp, state.objective_max) } else { String::new() };
    let stock = &state.stock;
    let ore = if stock.ore != [0; 3] || state.objective_max > 0.0 { format!("\nIRON {}  COPPER {}  AETHER {}", stock.ore[0], stock.ore[1], stock.ore[2]) } else { String::new() };
    let status = format!("{beacon}HEALTH {:.0}/{:.0}\nCREDITS ${}{ore}\n\n{weapon_line}", health.hp.max(0.0), health.max, stock.credits);
    let critical = (state.objective_max > 0.0 && state.objective_hp < state.objective_max * 0.25) || health.fraction() < 0.25;

    let squad = match &*link {
        NetLink::Host(host) => format!("\nHOSTING - {} GUEST(S)", host.peers.len()),
        NetLink::Client(_) => "\nONLINE".to_string(),
        NetLink::None => String::new(),
    };
    let wave = match state.phase {
        Phase::Day => format!("DAY {}\nNIGHTFALL IN {}\nKILLS {}{squad}", state.day, clock(state.timer), state.kills),
        Phase::Night => format!("NIGHT {}\nHOSTILES {}\nKILLS {}{squad}", state.day, state.enemies_left, state.kills),
        Phase::Won => format!("VICTORY\nKILLS {}", state.kills),
        Phase::Lost => format!("DEFEAT - NIGHT {}\nKILLS {}", state.day, state.kills),
    };

    let (use_key, fire_key, cancel_key, rotate_key) = if controls.gamepad { ("X", "RT", "B", "X") } else { ("E", "LMB", "ESC", "R") };
    let prompt = if !health.alive() {
        format!("YOU ARE DOWN\nREDEPLOYING IN {:.0}", player.respawn.ceil())
    } else if let Some(kind) = placing.kind {
        let name = content.structures.get(kind as usize).map_or("STRUCTURE", |s| s.name);
        format!("PLACING {name}\n[{fire_key}] BUILD   [{rotate_key}] ROTATE   [{cancel_key}] DONE")
    } else if let Some((_, kind)) = nearby.0 {
        format!("[{use_key}] {}", content.stations.get(kind as usize).map_or("STATION", |s| s.name))
    } else {
        String::new()
    };
    let hint = if controls.gamepad {
        "STICKS MOVE/LOOK   RT FIRE   LT AIM   X RELOAD/USE   A JUMP   LB/RB WEAPON   START PAUSE"
    } else {
        "WASD MOVE   LMB FIRE   RMB AIM   R RELOAD   E USE   1-8/WHEEL WEAPON   SHIFT SPRINT   ESC PAUSE"
    };
    let cross = if !health.alive() || view.ads > 0.85 { "" } else if view.hit > 0.0 { "X" } else { "+" };
    let notice = notices.0.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n");

    for (slot, mut text, mut color) in &mut texts {
        let (wanted, tint): (&str, Color) = match slot {
            Slot::Status => (&status, if critical { ui::WARN } else { ui::TEXT }),
            Slot::Wave => (&wave, if state.phase == Phase::Night { ui::WARN } else { ui::TEXT }),
            Slot::Cross => (cross, if view.hit > 0.0 && view.kill { ui::WARN } else { ui::TEXT }),
            Slot::Notice => (&notice, ui::TEXT),
            Slot::Prompt => (&prompt, ui::ACCENT),
            Slot::Hint => (hint, ui::DIM),
        };
        if text.0 != wanted {
            text.0 = wanted.to_string();
        }
        if color.0 != tint {
            color.0 = tint;
        }
    }
}
