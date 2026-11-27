//! The armory: five classes, each with two weapons and their attachments, a
//! perk and a piece of gear, edited on one screen with the gun turning
//! beside the lists.
//!
//! * **In the lobby** every class can be edited. Pick a class on the left,
//!   a tab (primary, secondary, perk, gear) along the top - LB / RB or
//!   PgUp / PgDn step through them - then a weapon from the list. The
//!   attachment rows step through the parts that fit with left / right
//!   (d-pad, stick or arrow keys), and the gun on the stage and the stat
//!   bars update as you browse, before you commit. B / Esc backs out one
//!   column at a time.
//! * **During a match** the classes are locked: you can only choose which
//!   one to spawn with next (L / D-pad down opens the picker).
//!
//! Classes are saved in the settings file. The chosen class goes to the
//! authority as an ordinary `Loadout` command, attachments included, so
//! the host validates it like any other order.

use crate::{
    ClientSet,
    input::Controls,
    settings::Settings,
    showroom::Stage,
    ui::{self, Act, ActEvent, FocusRequest, Focused, Order, Screen, UiDirty},
};
use bevy::{ecs::hierarchy::ChildSpawnerCommands, prelude::*};
use oz_core::arsenal::{SLOT_COUNT, SLOTS, WeaponDef, WeaponState, options, resolve};
use oz_core::kit::gun;
use oz_sim::prelude::*;

pub const CLASS_COUNT: usize = 5;
pub const CLASS_NAMES: [&str; CLASS_COUNT] = ["ASSAULT", "CLOSE QUARTERS", "MARKSMAN", "BREACHER", "RECON"];
const TABS: [&str; 5] = ["PRIMARY", "SECONDARY", "PERK", "GEAR", "STREAKS"];

/// The player's five classes: which one they spawn with, and the armory's place.
#[derive(Resource, Clone)]
pub struct Class {
    pub slots: [Loadout; CLASS_COUNT],
    /// The class to spawn with.
    pub active: usize,
    /// The armory's open tab: 0 primary, 1 secondary, 2 perk, 3 gear, 4 killstreaks.
    pub tab: u8,
    /// Which page of the armory is showing.
    pub page: Page,
    loaded: bool,
}

/// The armory shows one thing at a time and drills down: pick a class, then one of its
/// slots, then what goes in it, then (for a gun) its attachments. B / Esc goes back up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Classes,
    Class,
    Pick,
    Parts,
}

impl Default for Class {
    fn default() -> Self {
        let class = |primary: usize, secondary: usize, perk: u8, gear: u8| Loadout { primary: primary as u8, secondary: secondary as u8, perk, gear, parts: None, streaks: [0; 3] };
        Self {
            slots: [
                class(gun::CARBINE, gun::PISTOL, 0, 0),
                class(gun::SMG, gun::PISTOL, 2, 3),
                class(gun::DMR, gun::REVOLVER, 3, 2),
                class(gun::SHOTGUN, gun::PISTOL, 1, 0),
                class(gun::SNIPER, gun::PDW, 5, 4),
            ],
            active: 0,
            tab: 0,
            page: Page::Classes,
            loaded: false,
        }
    }
}

impl Class {
    /// The class this player spawns with.
    pub fn current(&self) -> Loadout {
        self.slots[self.active]
    }

    /// The weapon in a tab's slot (tabs 0 and 1) and its fitted parts.
    fn weapon(&self, tab: u8) -> (u8, [u8; SLOT_COUNT]) {
        let c = self.slots[self.active];
        let parts = c.parts.unwrap_or_default();
        if tab == 1 { (c.secondary, parts[1]) } else { (c.primary, parts[0]) }
    }
}

/// `primary,secondary,perk,gear;parts;parts`, one line per class in the settings file.
fn encode(loadout: &Loadout) -> String {
    let parts = loadout.parts.unwrap_or_default();
    let list = |p: &[u8; SLOT_COUNT]| p.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
    let streaks = loadout.streaks.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
    format!("{},{},{},{};{};{};{streaks}", loadout.primary, loadout.secondary, loadout.perk, loadout.gear, list(&parts[0]), list(&parts[1]))
}

fn decode(text: &str) -> Option<Loadout> {
    let mut sections = text.split(';');
    let head: Vec<u8> = sections.next()?.split(',').filter_map(|n| n.trim().parse().ok()).collect();
    let mut parts = [[0u8; SLOT_COUNT]; 2];
    for side in &mut parts {
        let values: Vec<u8> = sections.next()?.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        if values.len() != SLOT_COUNT {
            return None;
        }
        side.copy_from_slice(&values);
    }
    let mut streaks = [0u8; 3];
    if let Some(section) = sections.next() {
        for (slot, value) in streaks.iter_mut().zip(section.split(',').filter_map(|n| n.trim().parse().ok())) {
            *slot = value;
        }
    }
    (head.len() == 4).then(|| Loadout { primary: head[0], secondary: head[1], perk: head[2], gear: head[3], parts: Some(parts), streaks })
}

fn is_gun(content: &Content, index: u8) -> bool {
    content.weapons.get(index as usize).is_some_and(|w| w.frame.is_some())
}

/// The parts on `def` with any that do not fit replaced by its defaults.
fn fitted(def: &WeaponDef, content: &Content, parts: [u8; SLOT_COUNT]) -> [u8; SLOT_COUNT] {
    std::array::from_fn(|slot| {
        let fits = options(def, &content.attachments, slot).iter().any(|(i, ..)| *i == parts[slot] as usize);
        if fits { parts[slot] } else { def.defaults[slot] }
    })
}

/// Makes a class valid for this catalog: real guns, two different ones, parts that fit.
fn normalize(loadout: &mut Loadout, content: &Content) {
    let guns: Vec<u8> = (0..content.weapons.len() as u8).filter(|i| is_gun(content, *i)).collect();
    if guns.len() < 2 {
        return;
    }
    if !is_gun(content, loadout.primary) {
        loadout.primary = guns[1];
    }
    if !is_gun(content, loadout.secondary) || loadout.secondary == loadout.primary {
        loadout.secondary = guns.iter().copied().find(|g| *g != loadout.primary).unwrap_or(guns[0]);
    }
    loadout.perk = loadout.perk.min(content.perks.len().saturating_sub(1) as u8);
    loadout.gear = loadout.gear.min(content.gear.len().saturating_sub(1) as u8);
    let mut parts = loadout.parts.unwrap_or_else(|| [content.weapons[loadout.primary as usize].defaults, content.weapons[loadout.secondary as usize].defaults]);
    parts[0] = fitted(&content.weapons[loadout.primary as usize], content, parts[0]);
    parts[1] = fitted(&content.weapons[loadout.secondary as usize], content, parts[1]);
    loadout.parts = Some(parts);
    loadout.streaks = oz_sim::killstreak::chosen(loadout.streaks);
}

fn save(class: &Class, settings: &mut Settings) {
    settings.classes = class.slots.iter().map(encode).collect();
    settings.save();
}

/// Label for loadout input; the gunsmith runs after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoadoutInput;

pub struct LoadoutPlugin;

impl Plugin for LoadoutPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Class>()
            .add_systems(
                Update,
                (load, toggle, armory_actions, send_class, hold_primary)
                    .chain()
                    .in_set(LoadoutInput)
                    .in_set(ClientSet::Input)
                    .after(crate::input::ReadDevices)
                    .before(crate::menu::MenuActions)
                    .run_if(in_state(MatchPhase::Active)),
            )
            .add_systems(Update, preview.after(crate::menu::BuildScreens).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

/// Reads the saved classes once, and fits them to the match's catalog.
fn load(mut class: ResMut<Class>, settings: Res<Settings>, content: Res<Content>, career: Res<crate::career::Career>) {
    if !class.loaded {
        class.loaded = true;
        for (slot, text) in class.slots.iter_mut().zip(&settings.classes) {
            if let Some(loadout) = decode(text) {
                *slot = loadout;
            }
        }
    }
    if content.is_changed() || career.is_changed() || class.slots.iter().any(|s| s.parts.is_none()) {
        // Valid for this catalog, and only what this career has unlocked.
        for slot in class.slots.iter_mut() {
            normalize(slot, &content);
            career.enforce(slot, &content);
        }
    }
}

fn toggle(controls: Res<Controls>, rules: Res<Rules>, state: Res<MatchState>, mut screen: ResMut<Screen>, mut request: ResMut<FocusRequest>, mut class: ResMut<Class>) {
    if controls.loadout && rules.pvp && *screen == Screen::None && state.phase == Phase::Live {
        *screen = Screen::Loadout;
        request.0 = Some(Act::ClassPick(class.active as u8));
    }
    if *screen != Screen::Loadout && class.page != Page::Classes {
        class.page = Page::Classes;
    }
}

/// The armory edits only between matches; during one it only picks.
fn editable(state: &MatchState) -> bool {
    state.phase != Phase::Live
}

#[allow(clippy::too_many_arguments)]
fn armory_actions(
    mut events: EventReader<ActEvent>,
    mut screen: ResMut<Screen>,
    mut class: ResMut<Class>,
    mut dirty: ResMut<UiDirty>,
    mut settings: ResMut<Settings>,
    mut request: ResMut<FocusRequest>,
    (content, career): (Res<Content>, Res<crate::career::Career>),
    state: Res<MatchState>,
    mut players: Query<(&Player, &mut PlayerInput), With<LocalPlayer>>,
    mut orders: EventWriter<CommandEvent>,
    mut fx: EventWriter<FxEvent>,
    mut sounds: EventWriter<crate::fx::PlaySfx>,
) {
    if *screen != Screen::Loadout {
        return;
    }
    let Ok((player, mut input)) = players.single_mut() else {
        return;
    };
    let edit = editable(&state);
    let mut changed = false;
    let mut leave = |class: &mut Class, settings: &mut Settings, screen: &mut Screen, send: bool| {
        if send {
            let mut loadout = class.current();
            career.enforce(&mut loadout, &content);
            orders.write(CommandEvent { player: player.id, command: PlayerCommand::Loadout(loadout) });
            input.weapon = loadout.primary;
        }
        class.page = Page::Classes;
        save(class, settings);
        *screen = if state.phase == Phase::Lobby { Screen::Lobby } else { Screen::None };
    };
    for ActEvent(act) in events.read() {
        let active = class.active;
        match act {
            Act::ClassPick(index) => {
                class.active = (*index as usize).min(CLASS_COUNT - 1);
                if !edit {
                    // Mid-match: this is the class for the next life.
                    fx.write(FxEvent::tell(player.id, format!("{} - NEXT SPAWN", CLASS_NAMES[class.active])));
                    leave(&mut class, &mut settings, &mut screen, true);
                    return;
                }
                class.page = Page::Class;
                request.0 = Some(Act::ArmoryTab(0));
                changed = true;
                dirty.0 = true;
            }
            Act::ArmoryTab(tab) if edit => {
                class.tab = (*tab).min(4);
                class.page = Page::Pick;
                request.0 = Some(tab_focus(&class));
                dirty.0 = true;
            }
            Act::Tab(step) if edit && matches!(class.page, Page::Pick | Page::Parts) => {
                class.tab = (class.tab as i32 + step).rem_euclid(TABS.len() as i32) as u8;
                class.page = Page::Pick;
                request.0 = Some(tab_focus(&class));
                dirty.0 = true;
            }
            Act::ArmoryItem(item) if edit => {
                let tab = class.tab;
                let slot = &mut class.slots[active];
                let mut parts = slot.parts.unwrap_or_default();
                match tab {
                    0 | 1 if is_gun(&content, *item) && !career.gun_unlocked(*item) => {
                        sounds.write(crate::fx::PlaySfx::ui(Sfx::UiDeny));
                        continue;
                    }
                    0 | 1 if is_gun(&content, *item) => {
                        let (mine, other) = if tab == 0 { (slot.primary, slot.secondary) } else { (slot.secondary, slot.primary) };
                        let side = tab as usize;
                        if *item == other {
                            // Already in the other slot: the two swap places, parts and all.
                            parts.swap(0, 1);
                            if tab == 0 { slot.secondary = mine } else { slot.primary = mine }
                        } else if *item != mine {
                            parts[side] = content.weapons[*item as usize].defaults;
                        }
                        if tab == 0 {
                            slot.primary = *item
                        } else {
                            slot.secondary = *item
                        }
                        slot.parts = Some(parts);
                        // On to the attachments for this gun.
                        class.page = Page::Parts;
                        request.0 = Some(Act::ArmoryPart(0, 1));
                    }
                    2 | 3 => {
                        if tab == 2 {
                            slot.perk = (*item).min(content.perks.len().saturating_sub(1) as u8);
                        } else {
                            slot.gear = (*item).min(content.gear.len().saturating_sub(1) as u8);
                        }
                        // Chosen: back to the class.
                        class.page = Page::Class;
                        request.0 = Some(Act::ArmoryTab(tab));
                    }
                    4 if oz_sim::killstreak::def(*item).is_some() => {
                        // Toggle: up to three, at least one, kept cheapest first.
                        let mut picked: Vec<u8> = slot.streaks.iter().copied().filter(|id| *id != 0).collect();
                        if let Some(at) = picked.iter().position(|id| id == item) {
                            if picked.len() > 1 {
                                picked.remove(at);
                            }
                        } else {
                            if picked.len() >= 3 {
                                picked.pop();
                            }
                            picked.push(*item);
                        }
                        picked.sort_by_key(|id| oz_sim::killstreak::def(*id).map_or(0, |d| d.kills));
                        slot.streaks = [0; 3];
                        for (s, id) in slot.streaks.iter_mut().zip(picked) {
                            *s = id;
                        }
                    }
                    _ => {}
                }
                changed = true;
                dirty.0 = true;
            }
            Act::ArmoryPart(part, step) if edit => {
                let side = class.tab.min(1) as usize;
                let (weapon, mut parts) = class.weapon(side as u8);
                let Some(def) = content.weapons.get(weapon as usize) else {
                    continue;
                };
                // Only what this gun's kills have unlocked.
                let choices: Vec<_> = options(def, &content.attachments, *part as usize).into_iter().filter(|(i, ..)| career.part_unlocked(def, &content, weapon, *part as usize, *i as u8)).collect();
                if choices.len() > 1 {
                    let at = choices.iter().position(|(i, ..)| *i == parts[*part as usize] as usize).unwrap_or(0) as i32;
                    parts[*part as usize] = choices[(at + step).rem_euclid(choices.len() as i32) as usize].0 as u8;
                    let slot = &mut class.slots[active];
                    let mut all = slot.parts.unwrap_or_default();
                    all[side] = parts;
                    slot.parts = Some(all);
                    changed = true;
                    dirty.0 = true;
                }
            }
            Act::Back if edit && class.page != Page::Classes => {
                // Up one page, landing on what we came from.
                let (page, focus) = match class.page {
                    Page::Parts => (Page::Pick, tab_focus(&class)),
                    Page::Pick => (Page::Class, Act::ArmoryTab(class.tab)),
                    _ => (Page::Classes, Act::ClassPick(class.active as u8)),
                };
                class.page = page;
                request.0 = Some(focus);
                dirty.0 = true;
            }
            Act::Deploy | Act::Back => {
                leave(&mut class, &mut settings, &mut screen, edit);
                return;
            }
            _ => {}
        }
    }
    if changed {
        save(&class, &mut settings);
    }
}

/// Where focus goes when a slot's list opens: the item it holds now.
fn tab_focus(class: &Class) -> Act {
    let c = class.slots[class.active];
    Act::ArmoryItem(match class.tab {
        0 => c.primary,
        1 => c.secondary,
        2 => c.perk,
        3 => c.gear,
        _ => c.streaks[0],
    })
}

/// Before a match starts the host always has the class you will spawn with.
fn send_class(
    class: Res<Class>,
    state: Res<MatchState>,
    rules: Res<Rules>,
    players: Query<(Entity, &Player), With<LocalPlayer>>,
    mut orders: EventWriter<CommandEvent>,
    mut sent: Local<Option<(Entity, Loadout)>>,
) {
    let Ok((entity, player)) = players.single() else {
        return;
    };
    if !rules.pvp || state.phase != Phase::Lobby || class.slots.iter().any(|s| s.parts.is_none()) {
        return;
    }
    let loadout = class.current();
    if *sent != Some((entity, loadout)) {
        *sent = Some((entity, loadout));
        orders.write(CommandEvent { player: player.id, command: PlayerCommand::Loadout(loadout) });
    }
}

/// After a respawn the new class may have arrived: draw its primary.
fn hold_primary(class: Res<Class>, rules: Res<Rules>, mut players: Query<(&Health, &Arsenal, &mut PlayerInput), With<LocalPlayer>>, mut was_alive: Local<bool>) {
    let Ok((health, arsenal, mut input)) = players.single_mut() else {
        return;
    };
    let respawned = health.alive() && !*was_alive;
    *was_alive = health.alive();
    let primary = class.current().primary;
    if rules.pvp && (respawned || !arsenal.owns(input.weapon as usize)) && arsenal.owns(primary as usize) {
        input.weapon = primary;
    }
}

fn set_text<F: bevy::ecs::query::QueryFilter>(query: &mut Query<&mut Text, F>, value: String) {
    if let Ok(mut text) = query.single_mut()
        && text.0 != value
    {
        text.0 = value;
    }
}

#[derive(Component, Clone, Copy)]
struct ArmoryFill(usize);

#[derive(Component, Clone, Copy)]
struct ArmoryDelta(usize);

/// The description under the attachments: the focused weapon, part, perk or gear.
#[derive(Component)]
struct ArmoryBlurb;

/// The name of the gun being previewed.
#[derive(Component)]
struct ArmoryTitle;

fn panel_at(left: Option<f32>, right: Option<f32>, top: f32, width: f32) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: left.map_or(Val::Auto, Val::Px),
            right: right.map_or(Val::Auto, Val::Px),
            top: Val::Px(top),
            width: Val::Px(width),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            padding: UiRect::all(Val::Px(10.0)),
            border: UiRect::left(Val::Px(3.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.045, 0.065, 0.9)),
        BorderColor(ui::ACCENT),
    )
}

fn heading(parent: &mut ChildSpawnerCommands, text: impl Into<String>) {
    parent
        .spawn((Node { padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)), margin: UiRect::bottom(Val::Px(4.0)), ..default() }, BackgroundColor(Color::srgba(0.45, 0.88, 0.95, 0.16))))
        .with_children(|h| ui::label(h, text, 14.0, ui::ACCENT));
}

fn streak_names(loadout: &Loadout) -> String {
    loadout.streaks.iter().filter_map(|id| oz_sim::killstreak::def(*id)).map(|d| d.name).collect::<Vec<_>>().join(" / ")
}

/// What a class slot holds, as one line.
fn slot_value(content: &Content, loadout: &Loadout, tab: u8) -> String {
    let name = |i: u8| content.weapons.get(i as usize).map_or("-", |w| w.name).to_string();
    match tab {
        0 => name(loadout.primary),
        1 => name(loadout.secondary),
        2 => content.perks.get(loadout.perk as usize).map_or("-", |p| p.name).to_string(),
        3 => content.gear.get(loadout.gear as usize).map_or("-".to_string(), |g| format!("{} x{}", g.name, g.count)),
        _ => streak_names(loadout),
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
pub fn build_loadout(mut commands: Commands, dirty: Res<UiDirty>, screen: Res<Screen>, class: Res<Class>, controls: Res<Controls>, content: Option<Res<Content>>, state: Option<Res<MatchState>>, career: Res<crate::career::Career>) {
    let (Screen::Loadout, true, Some(content), Some(state)) = (&*screen, dirty.0, content, state) else {
        return;
    };
    let edit = editable(&state);
    let page = if edit { class.page } else { Page::Classes };
    let c = class.slots[class.active];
    let mut order = Order::default();
    let order = &mut order;
    let gun_page = matches!(page, Page::Pick | Page::Parts) && class.tab <= 1;

    ui::root(&mut commands, 0.0).with_children(|root| {
        // Where we are: ARMORY > ASSAULT > PRIMARY > ATTACHMENTS.
        root.spawn(Node { position_type: PositionType::Absolute, left: Val::Px(24.0), top: Val::Px(14.0), flex_direction: FlexDirection::Column, ..default() }).with_children(|top| {
            ui::label(top, "ARMORY", 30.0, ui::TEXT);
            let mut trail = vec![if edit { "CLASSES".to_string() } else { "SELECT A CLASS FOR YOUR NEXT LIFE".to_string() }];
            if page != Page::Classes {
                trail.push(format!("{}  {}", class.active + 1, CLASS_NAMES[class.active]));
            }
            if matches!(page, Page::Pick | Page::Parts) {
                trail.push(TABS[class.tab as usize].to_string());
            }
            if page == Page::Parts {
                trail.push("ATTACHMENTS".to_string());
            }
            ui::label(top, format!(">  {}", trail.join("  >  ")), 14.0, ui::ACCENT);
            let (division, points) = career.division();
            ui::label(top, format!("LEVEL {}  ({:.0}%)      RANK {division} {points}", career.level(), career.progress() * 100.0), 12.0, ui::DIM);
        });

        // One list at a time, on the left.
        root.spawn(panel_at(Some(24.0), None, 84.0, 400.0)).with_children(|panel| match page {
            Page::Classes => {
                heading(panel, "CLASSES");
                for (i, name) in CLASS_NAMES.iter().enumerate() {
                    ui::slim(panel, order, format!("{}   {name}", i + 1), Act::ClassPick(i as u8), None, i == class.active, Val::Percent(100.0));
                }
                if !edit {
                    ui::label(panel, "CLASSES ARE LOCKED DURING A MATCH. EDIT THEM IN THE LOBBY.", 11.0, ui::DIM);
                }
            }
            Page::Class => {
                heading(panel, CLASS_NAMES[class.active]);
                for (t, name) in TABS.iter().enumerate() {
                    ui::slim(panel, order, format!("{name:<11}{}", slot_value(&content, &c, t as u8)), Act::ArmoryTab(t as u8), None, false, Val::Percent(100.0));
                }
            }
            Page::Pick => {
                heading(panel, format!("CHOOSE {}", TABS[class.tab as usize]));
                match class.tab {
                    0 | 1 => {
                        let held = if class.tab == 0 { c.primary } else { c.secondary };
                        let other = if class.tab == 0 { c.secondary } else { c.primary };
                        for (i, def) in content.weapons.iter().enumerate().filter(|(_, w)| w.frame.is_some()) {
                            let tag = if !career.gun_unlocked(i as u8) {
                                format!("  LOCKED - LV {}", crate::career::gun_level(i as u8))
                            } else if i as u8 == other {
                                "  (SWAP)".to_string()
                            } else {
                                String::new()
                            };
                            ui::slim(panel, order, format!("{:<19}{:<8}{tag}", def.name, def.class.name()), Act::ArmoryItem(i as u8), None, i as u8 == held, Val::Percent(100.0));
                        }
                    }
                    2 => {
                        for (i, perk) in content.perks.iter().enumerate() {
                            ui::slim(panel, order, perk.name, Act::ArmoryItem(i as u8), None, i as u8 == c.perk, Val::Percent(100.0));
                        }
                    }
                    3 => {
                        for (i, gear) in content.gear.iter().enumerate() {
                            ui::slim(panel, order, format!("{}  x{}", gear.name, gear.count), Act::ArmoryItem(i as u8), None, i as u8 == c.gear, Val::Percent(100.0));
                        }
                    }
                    _ => {
                        for def in oz_sim::killstreak::STREAKS.iter() {
                            ui::slim(panel, order, format!("{:<20}{:>2} KILLS", def.name, def.kills), Act::ArmoryItem(def.id), None, c.streaks.contains(&def.id), Val::Percent(100.0));
                        }
                        ui::label(panel, "PICK UP TO THREE. BIGGER REWARDS NEED LONGER STREAKS.", 11.0, ui::DIM);
                    }
                }
            }
            Page::Parts => {
                let (weapon, parts) = class.weapon(class.tab);
                if let Some(def) = content.weapons.get(weapon as usize) {
                    heading(panel, format!("{}   -   {} KILLS", def.name, career.kills_with(weapon)));
                    for (s, name) in SLOTS.iter().enumerate() {
                        let choices = options(def, &content.attachments, s);
                        let fitted = choices.iter().find(|(i, ..)| *i == parts[s] as usize).map_or("-", |(_, n, ..)| n);
                        let open = choices.iter().filter(|(i, ..)| career.part_unlocked(def, &content, weapon, s, *i as u8)).count();
                        if choices.len() > 1 {
                            let count = if open < choices.len() { format!("  {open}/{} UNLOCKED", choices.len()) } else { String::new() };
                            ui::slim(panel, order, format!("{name:<12}<  {fitted}  >{count}"), Act::ArmoryPart(s as u8, 1), Some(Act::ArmoryPart(s as u8, -1)), false, Val::Percent(100.0));
                        } else {
                            ui::disabled(panel, format!("{name:<12}{fitted}"), Val::Percent(100.0));
                        }
                    }
                }
            }
        });

        // Bottom right: what the thing under the cursor is, and for guns, what it does.
        root.spawn(panel_at(None, Some(24.0), 0.0, 400.0))
            .insert(Node {
                position_type: PositionType::Absolute,
                right: Val::Px(24.0),
                bottom: Val::Px(56.0),
                width: Val::Px(400.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::left(Val::Px(3.0)),
                ..default()
            })
            .with_children(|panel| {
                panel.spawn((ArmoryTitle, Text::new(""), TextFont { font_size: 15.0, ..default() }, TextColor(ui::TEXT)));
                panel.spawn((ArmoryBlurb, Text::new(""), TextFont { font_size: 12.0, ..default() }, TextColor(ui::DIM)));
                if gun_page || page == Page::Class {
                    for (index, name) in crate::gunsmith::BARS.iter().enumerate() {
                        panel.spawn(ui::row(8.0)).with_children(|row| {
                            row.spawn((Text::new(*name), TextFont { font_size: 10.0, ..default() }, TextColor(ui::DIM), Node { width: Val::Px(80.0), ..default() }));
                            row.spawn((Node { width: Val::Px(280.0), height: Val::Px(7.0), ..default() }, BackgroundColor(Color::srgba(0.1, 0.14, 0.17, 0.9)))).with_children(|track| {
                                track.spawn((ArmoryFill(index), Node { position_type: PositionType::Absolute, height: Val::Percent(100.0), ..default() }, BackgroundColor(ui::TEXT)));
                                track.spawn((ArmoryDelta(index), Node { position_type: PositionType::Absolute, height: Val::Percent(100.0), ..default() }, BackgroundColor(ui::GOOD)));
                            });
                        });
                    }
                }
            });

        // Footer: button prompts.
        root.spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(14.0),
            left: Val::Px(24.0),
            right: Val::Px(24.0),
            column_gap: Val::Px(14.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|bottom| {
            let pad = controls.gamepad;
            let hint = match (page, edit) {
                (_, false) => {
                    if pad {
                        "(A) SPAWN WITH THIS CLASS    (B) CLOSE"
                    } else {
                        "ENTER / CLICK: SPAWN WITH THIS CLASS    ESC CLOSE"
                    }
                }
                (Page::Classes, true) => {
                    if pad {
                        "(A) EDIT CLASS    (B) DONE"
                    } else {
                        "ENTER / CLICK: EDIT CLASS    ESC DONE"
                    }
                }
                (Page::Class, true) => {
                    if pad {
                        "(A) CHANGE    (B) BACK"
                    } else {
                        "ENTER / CLICK: CHANGE    ESC BACK"
                    }
                }
                (Page::Pick, true) => {
                    if pad {
                        "(A) CHOOSE    (B) BACK    LB / RB  NEXT SLOT    RIGHT STICK  TURN"
                    } else {
                        "ENTER / CLICK: CHOOSE    ESC BACK    PGUP / PGDN  NEXT SLOT    DRAG TO TURN"
                    }
                }
                (Page::Parts, true) => {
                    if pad {
                        "D-PAD LEFT / RIGHT  CHANGE PART    (B) BACK    RIGHT STICK  TURN"
                    } else {
                        "LEFT / RIGHT OR CLICK: CHANGE PART    ESC BACK    DRAG TO TURN"
                    }
                }
            };
            ui::label(bottom, hint, 12.0, ui::DIM);
            bottom.spawn(Node { flex_grow: 1.0, ..default() });
            let done = if !edit {
                "CLOSE"
            } else if state.phase == Phase::Lobby {
                "SAVE CLASSES"
            } else {
                "DEPLOY"
            };
            ui::sized_button(bottom, order, done, Act::Deploy, None, Val::Px(180.0));
        });
    });
}

/// Every frame: put whatever is under the cursor on the stage and in the card.
#[allow(clippy::too_many_arguments)]
fn preview(
    screen: Res<Screen>,
    focused: Res<Focused>,
    class: Res<Class>,
    content: Res<Content>,
    state: Res<MatchState>,
    players: Query<&Player, With<LocalPlayer>>,
    mut stage: ResMut<Stage>,
    mut fills: Query<(&ArmoryFill, &mut Node), Without<ArmoryDelta>>,
    mut deltas: Query<(&ArmoryDelta, &mut Node, &mut BackgroundColor), Without<ArmoryFill>>,
    mut blurb: Query<&mut Text, (With<ArmoryBlurb>, Without<ArmoryTitle>)>,
    mut title: Query<&mut Text, (With<ArmoryTitle>, Without<ArmoryBlurb>)>,
) {
    if *screen != Screen::Loadout {
        return;
    }
    let team = players.single().map_or(0, |p| p.team);
    let edit = editable(&state);
    let page = if edit { class.page } else { Page::Classes };
    let operator = |index: usize| {
        let c = class.slots[index];
        Stage::Operator { team, weapon: Some((c.primary, c.parts.unwrap_or_default()[0])) }
    };

    // The class pages: the operator holding the class's primary, and the class in words.
    let class_view = page == Page::Classes || (page == Page::Class && !matches!(focused.0, Some(Act::ArmoryTab(0 | 1))));
    if class_view {
        let index = match (&focused.0, page) {
            (Some(Act::ClassPick(i)), Page::Classes) => *i as usize,
            _ => class.active,
        }
        .min(CLASS_COUNT - 1);
        let wanted = operator(index);
        if *stage != wanted {
            *stage = wanted;
        }
        let c = class.slots[index];
        let (head, body) = match (&focused.0, page) {
            (Some(Act::ArmoryTab(t)), Page::Class) if *t >= 2 => {
                let text = match t {
                    2 => content.perks.get(c.perk as usize).map_or(String::new(), |p| p.blurb.to_string()),
                    3 => content.gear.get(c.gear as usize).map_or(String::new(), |g| g.blurb.to_string()),
                    _ => c.streaks.iter().filter_map(|id| oz_sim::killstreak::def(*id)).map(|d| format!("{:>2} KILLS  {}", d.kills, d.name)).collect::<Vec<_>>().join("\n"),
                };
                (format!("{}: {}", TABS[*t as usize], slot_value(&content, &c, *t)), text)
            }
            _ => {
                let lines: Vec<String> = (0..5u8).map(|t| format!("{:<10} {}", TABS[t as usize], slot_value(&content, &c, t))).collect();
                (format!("{}  {}", index + 1, CLASS_NAMES[index]), lines.join("\n"))
            }
        };
        set_text(&mut title, head);
        set_text(&mut blurb, body);
        if page == Page::Class {
            // Bars for the primary while a non-gun slot is focused.
            let (weapon, parts) = class.weapon(0);
            show_bars(&content, weapon, parts, weapon, parts, &mut fills, &mut deltas);
        }
        return;
    }

    // Perk, gear and streak lists: the operator, and the description of the focused item.
    if matches!(page, Page::Pick) && class.tab >= 2 {
        let wanted = operator(class.active);
        if *stage != wanted {
            *stage = wanted;
        }
        let (head, body) = match (&focused.0, class.tab) {
            (Some(Act::ArmoryItem(i)), 2) => content.perks.get(*i as usize).map_or((String::new(), String::new()), |p| (p.name.to_string(), p.blurb.to_string())),
            (Some(Act::ArmoryItem(i)), 3) => content.gear.get(*i as usize).map_or((String::new(), String::new()), |g| (format!("{}  x{}", g.name, g.count), g.blurb.to_string())),
            (Some(Act::ArmoryItem(i)), _) => oz_sim::killstreak::def(*i).map_or((String::new(), String::new()), |d| (format!("{}  -  {} KILLS", d.name, d.kills), d.blurb.to_string())),
            _ => (String::new(), String::new()),
        };
        set_text(&mut title, head);
        set_text(&mut blurb, body);
        return;
    }

    // Guns: the one under the cursor turning on the stage, with its numbers against what it replaces.
    let side = match (&focused.0, page) {
        (Some(Act::ArmoryTab(t)), Page::Class) => *t,
        _ => class.tab,
    }
    .min(1);
    let (current, current_parts) = class.weapon(side);
    let (weapon, parts, description) = match &focused.0 {
        Some(Act::ArmoryItem(i)) if is_gun(&content, *i) => {
            let def = &content.weapons[*i as usize];
            let parts = if *i == current { current_parts } else { def.defaults };
            (*i, parts, format!("{}  -  {}", def.class.name(), def.blurb))
        }
        Some(Act::ArmoryPart(s, _)) => {
            let def = &content.weapons[current as usize];
            let text = options(def, &content.attachments, *s as usize)
                .into_iter()
                .find(|(i, ..)| *i == current_parts[*s as usize] as usize)
                .map_or(String::new(), |(_, name, text, _)| format!("{name}: {text}"));
            (current, current_parts, text)
        }
        _ => (current, current_parts, content.weapons.get(current as usize).map_or(String::new(), |d| d.blurb.to_string())),
    };
    let wanted = Stage::Armory { weapon, parts };
    if *stage != wanted {
        *stage = wanted;
    }
    let Some(def) = content.weapons.get(weapon as usize) else {
        return;
    };
    let stats = resolve(def, &WeaponState { equipped: parts, ..default() }, &content.attachments);
    let damage = if stats.pellets > 1 { format!("{:.0}x{}", stats.damage, stats.pellets) } else { format!("{:.0}", stats.damage) };
    set_text(&mut title, format!("{}      DMG {damage}   {:.0} RPM   MAG {}", def.name, 60.0 / stats.interval.max(0.01), stats.mag));
    set_text(&mut blurb, description);
    show_bars(&content, current, current_parts, weapon, parts, &mut fills, &mut deltas);
}

/// Stat bars for `then`, with the difference from `now` in green (better) or red (worse).
fn show_bars(
    content: &Content,
    now_weapon: u8,
    now_parts: [u8; SLOT_COUNT],
    weapon: u8,
    parts: [u8; SLOT_COUNT],
    fills: &mut Query<(&ArmoryFill, &mut Node), Without<ArmoryDelta>>,
    deltas: &mut Query<(&ArmoryDelta, &mut Node, &mut BackgroundColor), Without<ArmoryFill>>,
) {
    let (Some(a), Some(b)) = (content.weapons.get(now_weapon as usize), content.weapons.get(weapon as usize)) else {
        return;
    };
    let state_of = |parts: [u8; SLOT_COUNT]| WeaponState { equipped: parts, ..default() };
    let now = crate::gunsmith::bars(&resolve(a, &state_of(now_parts), &content.attachments));
    let then = crate::gunsmith::bars(&resolve(b, &state_of(parts), &content.attachments));
    for (fill, mut node) in fills.iter_mut() {
        let width = Val::Percent(now[fill.0].min(then[fill.0]) * 100.0);
        if node.width != width {
            node.width = width;
        }
    }
    for (delta, mut node, mut color) in deltas.iter_mut() {
        let (x, y) = (now[delta.0], then[delta.0]);
        node.left = Val::Percent(x.min(y) * 100.0);
        node.width = Val::Percent((y - x).abs() * 100.0);
        color.0 = if y >= x { ui::GOOD } else { ui::WARN };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_round_trip_through_text() {
        let loadout = Loadout { primary: 3, secondary: 0, perk: 4, gear: 2, parts: Some([[1, 0, 2, 0, 1, 0, 3], [0; SLOT_COUNT]]), streaks: [1, 3, 5] };
        assert_eq!(decode(&encode(&loadout)), Some(loadout));
        assert_eq!(decode("garbage"), None);
    }
}
