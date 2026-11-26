//! The versus class editor: two weapons, free attachments, a perk and gear.
//!
//! Opens automatically when a versus match starts and any time afterwards
//! (L / Select). Everything is sent to the authority as commands, so the
//! host validates classes exactly like purchases.

use crate::{
    ClientSet,
    input::Controls,
    ui::{self, Act, ActEvent, Order, Screen, UiDirty},
};
use bevy::prelude::*;
use oz_core::arsenal::SLOTS;
use oz_sim::prelude::*;

/// The class the player last built; kept between matches.
#[derive(Resource, Clone, Copy)]
pub struct Class(pub Loadout);

impl Default for Class {
    fn default() -> Self {
        Self(Loadout { primary: 3, secondary: 0, perk: 0, gear: 0 })
    }
}

pub struct LoadoutPlugin;

impl Plugin for LoadoutPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Class>()
            .add_systems(OnEnter(MatchPhase::Active), open_at_start.after(crate::menu::EnterMatch))
            .add_systems(
                Update,
                (toggle, loadout_actions, hold_primary).chain().in_set(ClientSet::Input).after(crate::input::ReadDevices).before(crate::menu::MenuActions).run_if(in_state(MatchPhase::Active)),
            );
    }
}

fn open_at_start(rules: Res<Rules>, mut screen: ResMut<Screen>) {
    if rules.pvp {
        *screen = Screen::Loadout;
    }
}

fn toggle(controls: Res<Controls>, rules: Res<Rules>, mut screen: ResMut<Screen>) {
    if controls.loadout && rules.pvp && *screen == Screen::None {
        *screen = Screen::Loadout;
    }
}

fn loadout_actions(
    mut events: EventReader<ActEvent>,
    mut screen: ResMut<Screen>,
    mut class: ResMut<Class>,
    mut dirty: ResMut<UiDirty>,
    content: Res<Content>,
    mut players: Query<(&Player, &mut PlayerInput), With<LocalPlayer>>,
    mut orders: EventWriter<CommandEvent>,
) {
    if *screen != Screen::Loadout {
        return;
    }
    let Ok((player, mut input)) = players.single_mut() else { return };
    let weapons = content.weapons.len().max(1) as i32;
    for ActEvent(act) in events.read() {
        match act {
            Act::Class(field, step) => {
                let cycle = |value: u8, len: i32| (value as i32 + step).rem_euclid(len.max(1)) as u8;
                let c = &mut class.0;
                match field {
                    0 => c.primary = cycle(c.primary, weapons),
                    1 => c.secondary = cycle(c.secondary, weapons),
                    2 => c.perk = cycle(c.perk, content.perks.len() as i32),
                    _ => c.gear = cycle(c.gear, content.gear.len() as i32),
                }
                // The two slots can never hold the same weapon.
                if c.primary == c.secondary {
                    if *field == 0 { c.secondary = cycle(c.secondary, weapons) } else { c.primary = cycle(c.primary, weapons) }
                }
                dirty.0 = true;
            }
            Act::Order(command) => {
                orders.write(CommandEvent { player: player.id, command: command.clone() });
            }
            Act::Deploy | Act::Back => {
                orders.write(CommandEvent { player: player.id, command: PlayerCommand::Loadout(class.0) });
                input.weapon = class.0.primary;
                *screen = Screen::None;
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
    players: Query<&Arsenal, With<LocalPlayer>>,
) {
    let (Screen::Loadout, true, Some(content), Ok(arsenal)) = (&*screen, dirty.0, content, players.single()) else { return };
    let c = class.0;
    let mut order = Order::default();
    let order = &mut order;
    let weapon_name = |index: u8| content.weapons.get(index as usize).map_or("-", |w| w.name);
    let primary = c.primary as usize;

    ui::root(&mut commands, 0.6).with_children(|root| {
        ui::label(root, "LOADOUT", 34.0, ui::TEXT);
        root.spawn(ui::panel(900.0)).with_children(|panel| {
            let mut pick = |panel: &mut bevy::ecs::hierarchy::ChildSpawnerCommands, field: u8, title: &str, value: &str, blurb: &str| {
                ui::stepper(panel, order, format!("{title:<11}<  {value}  >"), Act::Class(field, -1), Act::Class(field, 1));
                ui::label(panel, format!("    {blurb}"), 13.0, ui::DIM);
            };
            pick(panel, 0, "PRIMARY", weapon_name(c.primary), content.weapons.get(primary).map_or("", |w| w.blurb));
            pick(panel, 1, "SECONDARY", weapon_name(c.secondary), content.weapons.get(c.secondary as usize).map_or("", |w| w.blurb));
            if let Some(perk) = content.perks.get(c.perk as usize) {
                pick(panel, 2, "PERK", perk.name, perk.blurb);
            }
            if let Some(gear) = content.gear.get(c.gear as usize) {
                pick(panel, 3, "GEAR", gear.name, &format!("{}  (x{})", gear.blurb, gear.count));
            }
            ui::label(panel, format!("{} ATTACHMENTS", weapon_name(c.primary)), 15.0, ui::ACCENT);
            let equipped = arsenal.weapons.get(primary).map_or([0; 5], |w| w.equipped);
            for (slot, variants) in content.attachments.iter().enumerate().take(5) {
                panel.spawn(ui::row(6.0)).with_children(|row| {
                    ui::label(row, format!("{:<9}", SLOTS[slot]), 14.0, ui::DIM);
                    for (variant, part) in variants.iter().enumerate() {
                        let text = if equipped[slot] as usize == variant { format!("[{}]", part.name) } else { part.name.to_string() };
                        let command = PlayerCommand::EquipPart { weapon: c.primary, slot: slot as u8, variant: variant as u8 };
                        ui::sized_button(row, order, text, Act::Order(command), None, Val::Auto);
                    }
                });
            }
            ui::button(panel, order, "DEPLOY", Act::Deploy);
            let key = if controls.gamepad { "SELECT" } else { "L" };
            ui::label(panel, format!("PRESS {key} IN A MATCH TO CHANGE CLASS. IT APPLIES ON YOUR NEXT SPAWN."), 13.0, ui::DIM);
        });
    });
}
