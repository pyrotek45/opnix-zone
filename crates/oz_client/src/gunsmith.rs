//! The gunsmith: customise a weapon slot by slot while it turns on the
//! showroom stage. Hovering (or focusing with a pad) any part previews it on
//! the 3D model and in the stat bars before you commit; picking it sends an
//! ordinary `EquipPart` command, so the host still validates and charges.

use crate::{
    ClientSet,
    loadout::Class,
    showroom::Stage,
    ui::{self, Act, ActEvent, Back, Focused, Order, Screen, UiDirty},
};
use bevy::prelude::*;
use oz_core::arsenal::{Resolved, SLOT_COUNT, SLOTS, options, resolve};
use oz_sim::prelude::*;

pub const BARS: [&str; 8] = ["DAMAGE", "RANGE", "ACCURACY", "CONTROL", "HANDLING", "FIRE RATE", "MOBILITY", "MAGAZINE"];

#[derive(Component, Clone, Copy)]
struct StatFill(usize);

#[derive(Component, Clone, Copy)]
struct StatDelta(usize);

#[derive(Component)]
struct PartBlurb;

pub struct GunsmithPlugin;

impl Plugin for GunsmithPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, gunsmith_actions.in_set(ClientSet::Input).after(crate::input::ReadDevices).after(crate::loadout::LoadoutInput).after(crate::stations::StationInput).before(crate::menu::MenuActions).run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, (refresh.before(crate::menu::BuildScreens), preview.after(crate::menu::BuildScreens)).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

/// Each stat as 0..1 for the bars.
pub fn bars(stats: &Resolved) -> [f32; 8] {
    [
        stats.damage * stats.pellets as f32 / 180.0,
        stats.range / 180.0,
        1.0 - (stats.spread / 0.12).min(1.0) * 0.85,
        1.0 - (stats.recoil / 2.2).min(1.0),
        (stats.handling - 0.6) / 0.8,
        (1.0 / stats.interval.max(0.01)) / 15.0,
        (stats.mobility - 0.85) / 0.3,
        (stats.mag as f32 / 150.0).sqrt(),
    ]
    .map(|v| v.clamp(0.02, 1.0))
}

/// Weapons the gunsmith can switch between from here.
fn bench(content: &Content, arsenal: &Arsenal, rules: &Rules, class: &Class, from: Back) -> Vec<u8> {
    match from {
        Back::Loadout if rules.pvp => vec![class.0.primary, class.0.secondary],
        _ => (0..content.weapons.len() as u8).filter(|i| content.weapons[*i as usize].frame.is_some() && (rules.pvp || arsenal.owns(*i as usize))).collect(),
    }
}

fn gunsmith_actions(
    mut events: EventReader<ActEvent>,
    mut screen: ResMut<Screen>,
    content: Res<Content>,
    rules: Res<Rules>,
    class: Res<Class>,
    players: Query<(&Player, &Arsenal), With<LocalPlayer>>,
    mut orders: EventWriter<CommandEvent>,
) {
    let Ok((player, arsenal)) = players.single() else { return };
    for ActEvent(act) in events.read() {
        if let Act::OpenGunsmith(weapon, from) = act {
            *screen = Screen::Gunsmith { weapon: *weapon, slot: 0, from: *from };
            continue;
        }
        let Screen::Gunsmith { weapon, slot, from } = *screen else { continue };
        match act {
            Act::GunSlot(s) => *screen = Screen::Gunsmith { weapon, slot: *s, from },
            Act::Part(s, v) => {
                orders.write(CommandEvent { player: player.id, command: PlayerCommand::EquipPart { weapon, slot: *s, variant: *v } });
            }
            Act::Weapon(step) => {
                let list = bench(&content, arsenal, &rules, &class, from);
                if !list.is_empty() {
                    let at = list.iter().position(|w| *w == weapon).unwrap_or(0) as i32;
                    *screen = Screen::Gunsmith { weapon: list[(at + step).rem_euclid(list.len() as i32) as usize], slot, from };
                }
            }
            Act::Back => {
                *screen = match from {
                    Back::Loadout => Screen::Loadout,
                    Back::Station { kind, page } => Screen::Station { kind, page },
                }
            }
            Act::Tab(step) => *screen = Screen::Gunsmith { weapon, slot: (slot as i32 + step).rem_euclid(SLOT_COUNT as i32) as u8, from },
            _ => {}
        }
    }
}

fn track(parent: &mut ChildSpawnerCommands, index: usize) {
    parent
        .spawn((Node { width: Val::Px(170.0), height: Val::Px(10.0), ..default() }, BackgroundColor(Color::srgba(0.1, 0.14, 0.17, 0.9))))
        .with_children(|track| {
            track.spawn((StatFill(index), Node { position_type: PositionType::Absolute, height: Val::Percent(100.0), ..default() }, BackgroundColor(ui::TEXT)));
            track.spawn((StatDelta(index), Node { position_type: PositionType::Absolute, height: Val::Percent(100.0), ..default() }, BackgroundColor(ui::GOOD)));
        });
}

use bevy::ecs::hierarchy::ChildSpawnerCommands;

#[allow(clippy::too_many_arguments)]
pub fn build_gunsmith(
    mut commands: Commands,
    dirty: Res<UiDirty>,
    screen: Res<Screen>,
    controls: Res<crate::input::Controls>,
    content: Option<Res<Content>>,
    rules: Option<Res<Rules>>,
    state: Option<Res<MatchState>>,
    players: Query<&Arsenal, With<LocalPlayer>>,
) {
    let Screen::Gunsmith { weapon, slot, .. } = *screen else { return };
    let (true, Some(content), Some(rules), Some(state), Ok(arsenal)) = (dirty.0, content, rules, state, players.single()) else { return };
    let Some(def) = content.weapons.get(weapon as usize) else { return };
    let Some(owned) = arsenal.weapons.get(weapon as usize) else { return };
    let mut order = Order::default();
    let order = &mut order;
    let free = rules.pvp;

    ui::root(&mut commands, 0.0).with_children(|root| {
        // Header: which gun, and how to switch.
        root.spawn(Node { position_type: PositionType::Absolute, top: Val::Px(18.0), width: Val::Percent(100.0), justify_content: JustifyContent::Center, column_gap: Val::Px(16.0), align_items: AlignItems::Center, ..default() })
            .with_children(|top| {
                ui::label(top, "GUNSMITH", 30.0, ui::TEXT);
                ui::sized_button(top, order, format!("<  {}  >", def.name), Act::Weapon(1), Some(Act::Weapon(-1)), Val::Auto);
                ui::label(top, def.class.name(), 15.0, ui::ACCENT);
                if !free {
                    ui::label(top, format!("${}", state.stock.credits), 18.0, ui::GOOD);
                }
            });

        // Left: the seven slots with what is fitted.
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(80.0),
                width: Val::Px(250.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                padding: UiRect::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.05, 0.075, 0.88)),
        ))
        .with_children(|left| {
            for (s, name) in SLOTS.iter().enumerate() {
                let choices = options(def, &content.attachments, s);
                let fitted = choices.iter().find(|(i, ..)| *i == owned.equipped[s] as usize).map_or("-", |(_, n, ..)| n);
                let text = if choices.len() <= 1 { format!("{name}\n  {fitted}") } else { format!("{name}  ({})\n  {fitted}", choices.len()) };
                ui::chip(left, order, text, Act::GunSlot(s as u8), s == slot as usize, Val::Percent(100.0));
            }
        });

        // Right: options for the chosen slot, a description, and the stat bars.
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(20.0),
                top: Val::Px(80.0),
                width: Val::Px(380.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                padding: UiRect::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.05, 0.075, 0.88)),
        ))
        .with_children(|right| {
            ui::label(right, SLOTS[slot as usize], 18.0, ui::ACCENT);
            for (variant, name, _, cost) in options(def, &content.attachments, slot as usize) {
                let fitted = owned.equipped[slot as usize] as usize == variant;
                let status = if fitted {
                    "FITTED".to_string()
                } else if free || owned.has_part(slot as usize, variant) || cost == 0 {
                    String::new()
                } else {
                    format!("${cost}")
                };
                ui::chip(right, order, format!("{name:<18}{status}"), Act::Part(slot, variant as u8), fitted, Val::Percent(100.0));
            }
            right.spawn((PartBlurb, Text::new(""), TextFont { font_size: 14.0, ..default() }, TextColor(ui::DIM), Node { min_height: Val::Px(36.0), ..default() }));
            for (index, name) in BARS.iter().enumerate() {
                right.spawn(ui::row(10.0)).with_children(|row| {
                    row.spawn((Text::new(*name), TextFont { font_size: 13.0, ..default() }, TextColor(ui::DIM), Node { width: Val::Px(110.0), ..default() }));
                    track(row, index);
                });
            }
        });

        // Bottom: how to drive it.
        root.spawn(Node { position_type: PositionType::Absolute, bottom: Val::Px(18.0), width: Val::Percent(100.0), justify_content: JustifyContent::Center, column_gap: Val::Px(16.0), align_items: AlignItems::Center, ..default() })
            .with_children(|bottom| {
                let hint = if controls.gamepad { "RIGHT STICK ROTATE    LB / RB SLOT    A FIT    B BACK" } else { "DRAG THE GUN TO ROTATE    HOVER TO PREVIEW    CLICK TO FIT    ESC BACK" };
                ui::label(bottom, hint, 13.0, ui::DIM);
                ui::sized_button(bottom, order, "DONE", Act::Back, None, Val::Px(120.0));
            });
    });
}

/// Every frame: preview the focused part on the model and in the bars.
#[allow(clippy::too_many_arguments)]
fn preview(
    screen: Res<Screen>,
    focused: Res<Focused>,
    content: Res<Content>,
    mut stage: ResMut<Stage>,
    players: Query<&Arsenal, With<LocalPlayer>>,
    mut fills: Query<(&StatFill, &mut Node), Without<StatDelta>>,
    mut deltas: Query<(&StatDelta, &mut Node, &mut BackgroundColor), Without<StatFill>>,
    mut blurb: Query<&mut Text, With<PartBlurb>>,
) {
    let Screen::Gunsmith { weapon, .. } = *screen else { return };
    let (Ok(arsenal), Some(def)) = (players.single(), content.weapons.get(weapon as usize)) else { return };
    let Some(current) = arsenal.weapons.get(weapon as usize).copied() else { return };
    let mut trial = current;
    let mut description = String::new();
    if let Some(Act::Part(s, v)) = &focused.0 {
        trial.equipped[*s as usize] = *v;
        if let Some((_, name, text, _)) = options(def, &content.attachments, *s as usize).into_iter().find(|(i, ..)| *i == *v as usize) {
            description = format!("{name}: {text}");
        }
    }
    let wanted = Stage::Gun { weapon, parts: trial.equipped };
    if *stage != wanted {
        *stage = wanted;
    }
    let (now, then) = (bars(&resolve(def, &current, &content.attachments)), bars(&resolve(def, &trial, &content.attachments)));
    for (fill, mut node) in &mut fills {
        let width = Val::Percent(now[fill.0].min(then[fill.0]) * 100.0);
        if node.width != width {
            node.width = width;
        }
    }
    for (delta, mut node, mut color) in &mut deltas {
        let (a, b) = (now[delta.0], then[delta.0]);
        node.left = Val::Percent(a.min(b) * 100.0);
        node.width = Val::Percent((b - a).abs() * 100.0);
        color.0 = if b >= a { ui::GOOD } else { ui::WARN };
    }
    if let Ok(mut text) = blurb.single_mut()
        && text.0 != description {
            text.0 = description;
        }
}

/// Rebuilds the screen when a fitted part or the money changes, so FITTED and prices stay true.
fn refresh(
    screen: Res<Screen>,
    state: Res<MatchState>,
    players: Query<&Arsenal, With<LocalPlayer>>,
    mut dirty: ResMut<UiDirty>,
    mut last: Local<Option<(u8, [u8; SLOT_COUNT], [u8; SLOT_COUNT], u32)>>,
) {
    let Screen::Gunsmith { weapon, .. } = *screen else {
        *last = None;
        return;
    };
    let Some(owned) = players.single().ok().and_then(|a| a.weapons.get(weapon as usize).copied()) else { return };
    let signature = (weapon, owned.equipped, owned.purchased, state.stock.credits);
    if last.is_some_and(|l| l != signature) {
        dirty.0 = true;
    }
    *last = Some(signature);
}
