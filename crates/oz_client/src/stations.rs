//! Walk-up stations and build placement.
//!
//! Stand next to a station, press interact, and a window opens. The window is
//! generated from the catalog: each [`StationPage`] kind knows how to list
//! its wares, and every button simply sends a `PlayerCommand` to the
//! authority. Works identically offline, as host, and as a remote client.

use crate::{
    ClientSet,
    actors::Models,
    art::Art,
    input::Controls,
    ui::{self, Act, ActEvent, Order, Screen, UiDirty},
};
use bevy::prelude::*;
use oz_core::arsenal::{MOD_MAX_LEVEL, SLOTS};
use oz_sim::{orders::{BUILD_REACH, DRILL_SNAP}, prelude::*, structures::snap_yaw};

const STATION_REACH: f32 = 3.4;

/// The station the local player can currently use, as (entity, kind).
#[derive(Resource, Default)]
pub struct Nearby(pub Option<(Entity, u16)>);

/// Build mode: a structure ghost follows the crosshair until placed or cancelled.
#[derive(Resource, Default)]
pub struct Placement {
    pub kind: Option<u16>,
    yaw: f32,
    /// True for the frame placement was cancelled, so the same key does not also pause.
    pub just_closed: bool,
}

/// Which weapon the attachment and mod pages are showing.
#[derive(Resource, Default)]
pub struct Bench {
    weapon: u8,
}

#[derive(Component)]
struct Ghost(u16);

#[derive(Component)]
struct GhostMarker;

pub struct StationsPlugin;

impl Plugin for StationsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Nearby>()
            .init_resource::<Placement>()
            .init_resource::<Bench>()
            .add_systems(OnEnter(MatchPhase::Active), |mut placement: ResMut<Placement>| *placement = Placement::default())
            .add_systems(
                Update,
                (find_station, place_structures, station_actions, refresh_when_stale)
                    .chain()
                    .in_set(ClientSet::Input)
                    .after(crate::input::ReadDevices)
                    .before(crate::menu::MenuActions)
                    .run_if(in_state(MatchPhase::Active)),
            );
    }
}

fn find_station(
    controls: Res<Controls>,
    mut nearby: ResMut<Nearby>,
    mut screen: ResMut<Screen>,
    placing: Res<Placement>,
    players: Query<(&Transform, &Health), With<LocalPlayer>>,
    stations: Query<(Entity, &Transform, &Station)>,
) {
    let Ok((player, health)) = players.single() else { return };
    let closest = stations
        .iter()
        .map(|(entity, transform, station)| (transform.translation.distance(player.translation), entity, station.kind))
        .filter(|(distance, ..)| *distance < STATION_REACH && health.alive())
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, entity, kind)| (entity, kind));
    if nearby.0 != closest {
        nearby.0 = closest;
    }
    match (&*screen, closest) {
        (Screen::None, Some((_, kind))) if controls.interact && placing.kind.is_none() => *screen = Screen::Station { kind, page: 0 },
        // Walking away (or dying) closes the window.
        (Screen::Station { .. }, None) => *screen = Screen::None,
        _ => {}
    }
}

/// Where the crosshair meets the ground, clamped to build reach.
fn aim_point(map: &GameMap, feet: Vec3, input: &PlayerInput) -> Vec3 {
    let eye = feet + Vec3::Y * EYE;
    let dir = input.aim_dir();
    let hit = eye + dir * map.raycast(eye, dir, BUILD_REACH);
    let flat = (hit - feet).xz().clamp_length_max(BUILD_REACH - 1.0);
    map.ground(feet.x + flat.x, feet.z + flat.y)
}

#[allow(clippy::too_many_arguments)]
fn place_structures(
    mut commands: Commands,
    controls: Res<Controls>,
    screen: Res<Screen>,
    map: Res<Map>,
    content: Res<Content>,
    art: Res<Art>,
    models: Res<Models>,
    mut placing: ResMut<Placement>,
    players: Query<(&Transform, &PlayerInput, &Player, &Health), With<LocalPlayer>>,
    ores: Query<&Transform, (With<OreNode>, Without<Ghost>)>,
    mut ghosts: Query<(Entity, &Ghost, &mut Transform), Without<LocalPlayer>>,
    mut orders: EventWriter<CommandEvent>,
) {
    placing.bypass_change_detection().just_closed = false;
    let Ok((player, input, info, health)) = players.single() else { return };
    let cancel = controls.aim_pressed || controls.pause || controls.ui.back || !health.alive();
    if placing.kind.is_some() && *screen == Screen::None && cancel {
        placing.kind = None;
        placing.just_closed = true;
    }
    let Some(kind) = placing.kind.filter(|_| *screen == Screen::None) else {
        for (ghost, ..) in &ghosts {
            commands.entity(ghost).despawn();
        }
        return;
    };
    let Some(def) = content.structures.get(kind as usize) else { return };
    if controls.reload_pressed {
        placing.yaw = snap_yaw(placing.yaw + std::f32::consts::FRAC_PI_2);
    }

    let mut at = aim_point(&map, player.translation, input);
    if matches!(def.kind, StructureKind::Drill { .. }) {
        // Preview the snap the authority will apply.
        if let Some(node) = ores.iter().map(|t| t.translation).filter(|p| p.xz().distance(at.xz()) <= DRILL_SNAP).min_by(|a, b| a.xz().distance(at.xz()).total_cmp(&b.xz().distance(at.xz()))) {
            at = node;
        }
    }
    let pose = Transform::from_translation(at).with_rotation(Quat::from_rotation_y(placing.yaw));
    match ghosts.iter_mut().find(|(_, ghost, _)| ghost.0 == kind) {
        Some((_, _, mut transform)) => *transform = pose,
        None => {
            for (ghost, ..) in &ghosts {
                commands.entity(ghost).despawn();
            }
            let ghost = commands.spawn((MatchEntity, Ghost(kind), pose)).id();
            if let Some(baked) = models.structures.get(kind as usize) {
                art.spawn(&mut commands, ghost, baked);
            }
            // A glowing footprint so the ghost reads as "not built yet".
            commands.entity(ghost).with_children(|parent| {
                parent.spawn((
                    GhostMarker,
                    Mesh3d(art.primitive(Shape::Cylinder)),
                    MeshMaterial3d(art.paint(Paint::Green)),
                    Transform::from_xyz(0.0, 0.06, 0.0).with_scale(Vec3::new(def.half.x.max(def.half.z) * 2.6, 0.05, def.half.x.max(def.half.z) * 2.6)),
                    bevy::pbr::NotShadowCaster,
                ));
            });
        }
    }
    if controls.fire_pressed {
        orders.write(CommandEvent { player: info.id, command: PlayerCommand::Build { kind, x: at.x, z: at.z, yaw: placing.yaw } });
    }
}

fn station_actions(
    mut events: EventReader<ActEvent>,
    mut screen: ResMut<Screen>,
    mut placing: ResMut<Placement>,
    mut bench: ResMut<Bench>,
    mut dirty: ResMut<UiDirty>,
    content: Res<Content>,
    players: Query<(&Player, &Arsenal), With<LocalPlayer>>,
    mut orders: EventWriter<CommandEvent>,
) {
    let Screen::Station { kind, page } = *screen else { return };
    let Ok((player, arsenal)) = players.single() else { return };
    let pages = content.stations.get(kind as usize).map_or(1, |station| station.pages.len().max(1));
    for ActEvent(act) in events.read() {
        match act {
            Act::Order(command) => {
                orders.write(CommandEvent { player: player.id, command: command.clone() });
            }
            Act::Place(structure) => {
                placing.kind = Some(*structure);
                *screen = Screen::None;
            }
            Act::Page(index) => *screen = Screen::Station { kind, page: *index },
            Act::Tab(step) => *screen = Screen::Station { kind, page: (page as i32 + step).rem_euclid(pages as i32) as usize },
            Act::Weapon(step) => {
                let owned: Vec<u8> = (0..arsenal.weapons.len() as u8).filter(|i| arsenal.owns(*i as usize) && content.weapons[*i as usize].mag > 0).collect();
                if !owned.is_empty() {
                    let at = owned.iter().position(|i| *i == bench.weapon).unwrap_or(0) as i32;
                    bench.weapon = owned[(at + step).rem_euclid(owned.len() as i32) as usize];
                    dirty.0 = true;
                }
            }
            Act::Back => *screen = Screen::None,
            _ => {}
        }
    }
}

/// Rebuilds an open station window when prices, stock or ownership change.
fn refresh_when_stale(
    screen: Res<Screen>,
    state: Res<MatchState>,
    players: Query<&Arsenal, With<LocalPlayer>>,
    mut dirty: ResMut<UiDirty>,
    mut last: Local<Option<(Stock, Vec<u8>, Phase, i32, Vec<oz_core::arsenal::WeaponState>)>>,
) {
    if !matches!(*screen, Screen::Station { .. }) {
        *last = None;
        return;
    }
    let Ok(arsenal) = players.single() else { return };
    // Ammo counts change every shot but never matter to the shop, so blank them.
    let weapons = arsenal.weapons.iter().map(|w| oz_core::arsenal::WeaponState { ammo: 0, ..*w }).collect();
    let weapons: Vec<_> = weapons;
    let signature = (state.stock, state.upgrades.clone(), state.phase, (state.objective_hp / 10.0) as i32, weapons);
    if last.as_ref() != Some(&signature) {
        if last.is_some() {
            dirty.0 = true;
        }
        *last = Some(signature);
    }
}

fn price(cost: &Cost, stock: &Stock) -> String {
    if stock.can_afford(cost) { cost.label() } else { format!("{}  (SHORT)", cost.label()) }
}

#[allow(clippy::too_many_arguments)]
pub fn build_station(
    mut commands: Commands,
    dirty: Res<UiDirty>,
    screen: Res<Screen>,
    controls: Res<Controls>,
    content: Option<Res<Content>>,
    state: Option<Res<MatchState>>,
    mut bench: ResMut<Bench>,
    players: Query<&Arsenal, With<LocalPlayer>>,
) {
    let Screen::Station { kind, page } = *screen else { return };
    let (true, Some(content), Some(state), Ok(arsenal)) = (dirty.0, content, state, players.single()) else { return };
    let Some(station) = content.stations.get(kind as usize) else { return };
    let page = page.min(station.pages.len().saturating_sub(1));
    let stock = &state.stock;
    let mut order = Order::default();
    let order = &mut order;

    // The bench always shows a weapon the player owns and that takes parts.
    let benchable = |i: usize| arsenal.owns(i) && content.weapons[i].mag > 0;
    if !benchable(bench.weapon as usize) {
        bench.weapon = (0..content.weapons.len()).find(|i| benchable(*i)).unwrap_or(0) as u8;
    }
    let weapon = bench.weapon as usize;
    let (tab_keys, close_key) = if controls.gamepad { ("LB / RB", "B") } else { ("TAB", "ESC") };

    ui::root(&mut commands, 0.55).with_children(|root| {
        root.spawn(ui::panel(900.0)).with_children(|panel| {
            panel.spawn(Node { justify_content: JustifyContent::SpaceBetween, width: Val::Percent(100.0), ..default() }).with_children(|header| {
                ui::label(header, station.name, 30.0, ui::TEXT);
                ui::label(header, format!("${}   IRON {}   COPPER {}   AETHER {}", stock.credits, stock.ore[0], stock.ore[1], stock.ore[2]), 18.0, ui::GOOD);
            });
            ui::label(panel, station.blurb, 14.0, ui::DIM);
            if station.pages.len() > 1 {
                panel.spawn(ui::row(6.0)).with_children(|tabs| {
                    for (index, tab) in station.pages.iter().enumerate() {
                        let name = if index == page { format!("[ {} ]", tab.title()) } else { tab.title().to_string() };
                        ui::sized_button(tabs, order, name, Act::Page(index), None, Val::Auto);
                    }
                    ui::label(tabs, format!("{tab_keys} SWITCHES TABS"), 13.0, ui::DIM);
                });
            }

            match station.pages[page] {
                StationPage::Weapons => {
                    for (index, def) in content.weapons.iter().enumerate() {
                        let owned = arsenal.weapons.get(index).filter(|w| w.owned);
                        match owned {
                            None => ui::button(panel, order, format!("{:<20} BUY   {}", def.name, price(&def.price, stock)), Act::Order(PlayerCommand::BuyWeapon(index as u8))),
                            Some(w) if !def.infinite_reserve && def.mag > 0 && w.reserve < def.mag * 8 => ui::button(
                                panel,
                                order,
                                format!("{:<20} AMMO +{}   {}   (RESERVE {})", def.name, def.mag * 2, price(&Cost::credits(def.ammo_price), stock), w.reserve),
                                Act::Order(PlayerCommand::BuyAmmo(index as u8)),
                            ),
                            Some(_) => ui::disabled(panel, format!("{:<20} OWNED", def.name), Val::Percent(100.0)),
                        }
                        ui::label(panel, format!("    {}", def.blurb), 13.0, ui::DIM);
                    }
                }
                StationPage::Attachments => {
                    ui::stepper(panel, order, format!("WEAPON   <  {}  >", content.weapons[weapon].name), Act::Weapon(-1), Act::Weapon(1));
                    let owned = arsenal.weapons[weapon];
                    for (slot, variants) in content.attachments.iter().enumerate().take(5) {
                        ui::label(panel, SLOTS[slot], 14.0, ui::ACCENT);
                        panel.spawn(ui::row(6.0)).with_children(|row| {
                            for (variant, part) in variants.iter().enumerate() {
                                let text = if owned.equipped[slot] as usize == variant {
                                    format!("[{}]", part.name)
                                } else if owned.has_part(slot, variant) {
                                    part.name.to_string()
                                } else {
                                    format!("{} ${}", part.name, part.cost)
                                };
                                let command = PlayerCommand::EquipPart { weapon: weapon as u8, slot: slot as u8, variant: variant as u8 };
                                ui::sized_button(row, order, text, Act::Order(command), None, Val::Auto);
                            }
                        });
                    }
                }
                StationPage::Mods => {
                    ui::stepper(panel, order, format!("WEAPON   <  {}  >", content.weapons[weapon].name), Act::Weapon(-1), Act::Weapon(1));
                    let owned = arsenal.weapons[weapon];
                    let used = owned.mods.iter().flatten().count();
                    ui::label(panel, format!("MOD SLOTS {used}/{}   -   EACH MOD HAS {MOD_MAX_LEVEL} LEVELS", owned.mods.len()), 14.0, ui::ACCENT);
                    for kind in &content.mods {
                        let level = owned.mod_level(*kind);
                        let text = format!("{:<11} L{level}   {:<32}", kind.name(), kind.blurb());
                        if level >= MOD_MAX_LEVEL {
                            ui::disabled(panel, format!("{text} MAX"), Val::Percent(100.0));
                        } else if level == 0 && used >= owned.mods.len() {
                            ui::disabled(panel, format!("{text} NO FREE SLOT"), Val::Percent(100.0));
                        } else {
                            ui::button(panel, order, format!("{text} {}", price(&kind.cost(level), stock)), Act::Order(PlayerCommand::BuyMod { weapon: weapon as u8, kind: *kind }));
                        }
                    }
                }
                StationPage::Build => {
                    for (index, def) in content.structures.iter().enumerate() {
                        ui::button(panel, order, format!("{:<14} {}", def.name, price(&def.cost, stock)), Act::Place(index as u16));
                        ui::label(panel, format!("    {}", def.blurb), 13.0, ui::DIM);
                    }
                    ui::label(panel, "PICK A STRUCTURE, THEN AIM AT THE GROUND AND FIRE TO PLACE IT.", 13.0, ui::ACCENT);
                }
                StationPage::Upgrades => {
                    for (index, def) in content.upgrades.iter().enumerate() {
                        let level = state.upgrades.get(index).copied().unwrap_or(0);
                        let pips: String = (0..def.max_level).map(|i| if i < level { '#' } else { '-' }).collect();
                        let text = format!("{:<20} {pips}  {:<30}", def.name, def.blurb);
                        if level >= def.max_level {
                            ui::disabled(panel, format!("{text} MAX"), Val::Percent(100.0));
                        } else {
                            ui::button(panel, order, format!("{text} {}", price(&def.cost.scaled(level as u32), stock)), Act::Order(PlayerCommand::Upgrade(index as u8)));
                        }
                    }
                }
                StationPage::Mission => {
                    let phase = match state.phase {
                        Phase::Day => format!("DAY {} - NIGHTFALL IN {:.0} SECONDS", state.day, state.timer.max(0.0)),
                        Phase::Night => format!("NIGHT {} - {} HOSTILES REMAIN", state.day, state.enemies_left),
                        Phase::Won => "VICTORY".into(),
                        Phase::Lost => "THE LINE HAS FALLEN".into(),
                    };
                    ui::label(panel, phase, 18.0, ui::TEXT);
                    if state.objective_max > 0.0 {
                        ui::label(panel, format!("BEACON INTEGRITY {:.0} / {:.0}", state.objective_hp, state.objective_max), 18.0, ui::TEXT);
                    }
                    for (index, action) in content.mode_actions.iter().enumerate() {
                        ui::button(panel, order, format!("{:<18} {}", action.name, price(&action.cost, stock)), Act::Order(PlayerCommand::Mode(index as u8)));
                        ui::label(panel, format!("    {}", action.blurb), 13.0, ui::DIM);
                    }
                }
            }
            ui::button(panel, order, format!("CLOSE   ({close_key})"), Act::Back);
        });
    });
}
