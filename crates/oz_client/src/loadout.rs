//! The versus class editor: two weapons, a perk and gear, with the gunsmith
//! one click away. Your robot stands beside the panel holding the primary.
//!
//! Opens from the lobby ("CREATE A CLASS") and during a match (L / D-pad
//! down). Everything is sent to the authority as commands, so the host
//! validates classes exactly like purchases.

use crate::{
    ClientSet,
    input::Controls,
    ui::{self, Act, ActEvent, Back, Order, Screen, UiDirty},
};
use bevy::prelude::*;
use oz_sim::prelude::*;

/// The class the player last built; kept between matches.
#[derive(Resource, Clone, Copy)]
pub struct Class(pub Loadout);

impl Default for Class {
    fn default() -> Self {
        Self(Loadout { primary: oz_core::kit::gun::CARBINE as u8, secondary: oz_core::kit::gun::PISTOL as u8, perk: 0, gear: 0 })
    }
}

/// Label for loadout input; the gunsmith runs after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoadoutInput;

pub struct LoadoutPlugin;

impl Plugin for LoadoutPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Class>().add_systems(
            Update,
            (toggle, loadout_actions, hold_primary).chain().in_set(LoadoutInput).in_set(ClientSet::Input).after(crate::input::ReadDevices).before(crate::menu::MenuActions).run_if(in_state(MatchPhase::Active)),
        );
    }
}

fn toggle(controls: Res<Controls>, rules: Res<Rules>, state: Res<MatchState>, mut screen: ResMut<Screen>) {
    if controls.loadout && rules.pvp && *screen == Screen::None && state.phase == Phase::Live {
        *screen = Screen::Loadout;
    }
}

fn loadout_actions(
    mut events: EventReader<ActEvent>,
    mut screen: ResMut<Screen>,
    mut class: ResMut<Class>,
    mut dirty: ResMut<UiDirty>,
    content: Res<Content>,
    state: Res<MatchState>,
    mut players: Query<(&Player, &mut PlayerInput), With<LocalPlayer>>,
    mut orders: EventWriter<CommandEvent>,
) {
    if *screen != Screen::Loadout {
        return;
    }
    let Ok((player, mut input)) = players.single_mut() else { return };
    // Only real guns go in a class.
    let guns: Vec<u8> = (0..content.weapons.len() as u8).filter(|i| content.weapons[*i as usize].frame.is_some()).collect();
    for ActEvent(act) in events.read() {
        match act {
            Act::Class(field, step) => {
                let cycle_gun = |value: u8| {
                    let at = guns.iter().position(|g| *g == value).unwrap_or(0) as i32;
                    guns.get((at + step).rem_euclid(guns.len().max(1) as i32) as usize).copied().unwrap_or(value)
                };
                let cycle = |value: u8, len: usize| (value as i32 + step).rem_euclid(len.max(1) as i32) as u8;
                let c = &mut class.0;
                match field {
                    0 => c.primary = cycle_gun(c.primary),
                    1 => c.secondary = cycle_gun(c.secondary),
                    2 => c.perk = cycle(c.perk, content.perks.len()),
                    _ => c.gear = cycle(c.gear, content.gear.len()),
                }
                // The two slots can never hold the same weapon.
                if c.primary == c.secondary {
                    if *field == 0 { c.secondary = cycle_gun(c.secondary) } else { c.primary = cycle_gun(c.primary) }
                }
                dirty.0 = true;
            }
            Act::Deploy | Act::Back => {
                orders.write(CommandEvent { player: player.id, command: PlayerCommand::Loadout(class.0) });
                input.weapon = class.0.primary;
                *screen = if state.phase == Phase::Lobby { Screen::Lobby } else { Screen::None };
            }
            _ => {}
        }
    }
}

/// After a respawn the new class may have arrived: draw its primary.
fn hold_primary(class: Res<Class>, rules: Res<Rules>, mut players: Query<(&Health, &Arsenal, &mut PlayerInput), With<LocalPlayer>>, mut was_alive: Local<bool>) {
    let Ok((health, arsenal, mut input)) = players.single_mut() else { return };
    let respawned = health.alive() && !*was_alive;
    *was_alive = health.alive();
    if rules.pvp && (respawned || !arsenal.owns(input.weapon as usize)) && arsenal.owns(class.0.primary as usize) {
        input.weapon = class.0.primary;
    }
}

pub fn build_loadout(
    mut commands: Commands,
    dirty: Res<UiDirty>,
    screen: Res<Screen>,
    class: Res<Class>,
    controls: Res<Controls>,
    content: Option<Res<Content>>,
    state: Option<Res<MatchState>>,
) {
    let (Screen::Loadout, true, Some(content), Some(state)) = (&*screen, dirty.0, content, state) else { return };
    let c = class.0;
    let mut order = Order::default();
    let order = &mut order;
    let weapon = |index: u8| content.weapons.get(index as usize);

    ui::root_left(&mut commands, 0.0).with_children(|root| {
        ui::label(root, "CREATE A CLASS", 34.0, ui::TEXT);
        root.spawn(ui::panel(560.0)).with_children(|panel| {
            let mut pick = |panel: &mut bevy::ecs::hierarchy::ChildSpawnerCommands, field: u8, title: &str, value: &str, blurb: &str| {
                ui::stepper(panel, order, format!("{title:<11}<  {value}  >"), Act::Class(field, -1), Act::Class(field, 1));
                ui::label(panel, format!("    {blurb}"), 13.0, ui::DIM);
            };
            let describe = |index: u8| weapon(index).map_or(String::new(), |w| format!("{}  -  {}", w.class.name(), w.blurb));
            pick(panel, 0, "PRIMARY", weapon(c.primary).map_or("-", |w| w.name), &describe(c.primary));
            pick(panel, 1, "SECONDARY", weapon(c.secondary).map_or("-", |w| w.name), &describe(c.secondary));
            if let Some(perk) = content.perks.get(c.perk as usize) {
                pick(panel, 2, "PERK", perk.name, perk.blurb);
            }
            if let Some(gear) = content.gear.get(c.gear as usize) {
                pick(panel, 3, "GEAR", gear.name, &format!("{}  (x{})", gear.blurb, gear.count));
            }
            panel.spawn(ui::row(8.0)).with_children(|row| {
                ui::sized_button(row, order, "GUNSMITH: PRIMARY", Act::OpenGunsmith(c.primary, Back::Loadout), None, Val::Px(250.0));
                ui::sized_button(row, order, "GUNSMITH: SECONDARY", Act::OpenGunsmith(c.secondary, Back::Loadout), None, Val::Px(250.0));
            });
            ui::button(panel, order, if state.phase == Phase::Lobby { "SAVE CLASS" } else { "DEPLOY" }, Act::Deploy);
            let key = if controls.gamepad { "D-PAD DOWN" } else { "L" };
            ui::label(panel, format!("PRESS {key} IN A MATCH TO CHANGE CLASS. IT APPLIES ON YOUR NEXT SPAWN."), 13.0, ui::DIM);
        });
    });
}
