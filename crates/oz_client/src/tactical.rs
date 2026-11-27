//! Tactical HUD and hand state.
//!
//! * The killstreak stack (bottom right, above the ammo): up to three earned
//!   rewards, newest on top - that is the one the call-in button uses - and
//!   how many more kills the next one needs.
//! * The grenade in your hand: a fuse meter while it cooks, with the keys to
//!   throw it or put the pin back. The same state drives the first-person
//!   throwing arm (`view`).
//! * The scope's stabilizer (servo lock) meter.
//! * A vault prompt when there is a wall, sill or crate in front to go over.
//! * Terrorist hunt: hostiles left and the squad's lives.

use crate::{
    ClientSet,
    bindings::Action,
    input::{Controls, Steady},
    settings::Settings,
    ui::{self, Screen},
};
use bevy::prelude::*;
use oz_sim::{
    killstreak::{self, STACK},
    prelude::*,
};

/// What the local player's hands are doing besides holding the gun.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct Hands {
    /// Seconds the grenade in hand has been cooking, and its fuse.
    pub cooking: Option<f32>,
    pub fuse: f32,
    /// Whether the held gear goes off when the fuse runs out (frags do, knives do not).
    pub live: bool,
    /// Seconds since the last throw, pin-back or melee (for the arm animations).
    pub since_throw: f32,
    pub since_cancel: f32,
    pub since_melee: f32,
}

/// The radar's map layer: north up, the map image inside it slides so you stay centred.
#[derive(Component)]
pub struct MinimapPivot;

/// Your marker in the middle of the radar, turned to where you face.
#[derive(Component)]
pub struct RadarArrow;

#[derive(Component)]
struct MinimapImage {
    extent: f32,
}

#[derive(Component)]
struct StreakSlot(usize);

#[derive(Component)]
struct StreakText;

#[derive(Component)]
struct StreakPanel;

#[derive(Component)]
struct HandMeter;

#[derive(Component)]
struct HandFill;

#[derive(Component)]
struct HandText;

#[derive(Component)]
struct HuntText;

/// The chevron that hangs over a ledge you can vault right now.
#[derive(Component)]
struct LedgeArrow;

/// Where the vault chevron goes this frame (top of the ledge ahead).
#[derive(Resource, Default)]
struct Ledge(Option<Vec3>);

pub struct TacticalPlugin;

impl Plugin for TacticalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hands>()
            .init_resource::<Lamp>()
            .init_resource::<Ledge>()
            .add_systems(Update, (toggles, share_lamp).chain().in_set(ClientSet::Input).after(crate::input::ReadDevices).run_if(in_state(MatchPhase::Active)))
            .add_systems(OnEnter(MatchPhase::Active), (spawn, |mut hands: ResMut<Hands>| *hands = Hands { since_throw: 9.0, since_cancel: 9.0, since_melee: 9.0, ..default() }))
            .add_systems(Update, track_hands.in_set(ClientSet::Input).after(crate::input::DrivePlayer).run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, ((update, loot_prompt, point_at_ledge).chain(), minimap, dress_loot, spin_loot, sway_chutes).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

/// The fuse of the local player's gear, if it is something thrown and held.
pub fn held_gear(content: &Content, player: &Player) -> Option<(f32, bool)> {
    let gear = content.gear.get(player.gear as usize)?;
    let (_, shot) = oz_sim::combat::throw(&gear.kind, Vec3::NEG_Z)?;
    let knife = matches!(gear.kind, GearKind::Knife { .. });
    Some((shot.life, !knife && matches!(shot.effect, oz_sim::combat::Impact::Blast)))
}

/// Mirrors the authority's grenade rules locally so the hand and meter react at once online.
fn track_hands(
    time: Res<Time>,
    controls: Res<Controls>,
    screen: Res<Screen>,
    content: Res<Content>,
    state: Res<MatchState>,
    mut hands: ResMut<Hands>,
    players: Query<(&Player, &PlayerInput, &Health), With<LocalPlayer>>,
    mut was_held: Local<bool>,
) {
    let dt = time.delta_secs();
    hands.since_throw += dt;
    hands.since_cancel += dt;
    hands.since_melee += dt;
    let Ok((player, input, health)) = players.single() else {
        return;
    };
    let held = input.held(buttons::GEAR) && *screen == Screen::None && health.alive();
    let pressed = held && !*was_held;
    *was_held = held;
    if input.held(buttons::MELEE) && hands.cooking.is_none() && hands.since_melee > 0.7 {
        hands.since_melee = 0.0;
    }
    if !health.alive() || state.phase == Phase::Lobby {
        hands.cooking = None;
        return;
    }
    if pressed
        && hands.cooking.is_none()
        && player.gear_left > 0
        && let Some((fuse, live)) = held_gear(&content, player)
    {
        hands.cooking = Some(0.0);
        hands.fuse = fuse;
        hands.live = live;
    }
    if let Some(cooked) = hands.cooking {
        if controls.reload_pressed {
            hands.cooking = None;
            hands.since_cancel = 0.0;
        } else if !held || (hands.live && cooked + dt >= hands.fuse) {
            hands.cooking = None;
            hands.since_throw = 0.0;
        } else {
            hands.cooking = Some(cooked + dt);
        }
    }
}

fn spawn(mut commands: Commands) {
    // Killstreaks: bottom right, above the ammo counter.
    commands
        .spawn((
            MatchEntity,
            StreakPanel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(20.0),
                bottom: Val::Px(128.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::End,
                row_gap: Val::Px(3.0),
                ..default()
            },
            GlobalZIndex(-40),
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            for slot in 0..STACK {
                panel
                    .spawn((
                        StreakSlot(slot),
                        Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)), border: UiRect::left(Val::Px(3.0)), min_width: Val::Px(170.0), justify_content: JustifyContent::End, ..default() },
                        BackgroundColor(Color::srgba(0.02, 0.04, 0.06, 0.7)),
                        BorderColor(Color::srgba(0.3, 0.4, 0.42, 0.5)),
                    ))
                    .with_children(|row| {
                        row.spawn((Text::new(""), TextFont { font_size: 13.0, ..default() }, TextColor(ui::TEXT)));
                    });
            }
            panel.spawn((StreakText, Text::new(""), TextFont { font_size: 11.0, ..default() }, TextColor(ui::DIM), TextLayout::new_with_justify(JustifyText::Right)));
        });

    // The meter under the crosshair: grenade fuse or scope stabilizer.
    commands
        .spawn((
            MatchEntity,
            HandMeter,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                top: Val::Percent(56.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(3.0),
                ..default()
            },
            GlobalZIndex(-38),
            Visibility::Hidden,
        ))
        .with_children(|meter| {
            meter.spawn((Node { width: Val::Px(90.0), height: Val::Px(4.0), ..default() }, BackgroundColor(Color::srgba(0.1, 0.14, 0.17, 0.85)))).with_children(|track| {
                track.spawn((HandFill, Node { height: Val::Percent(100.0), width: Val::Percent(0.0), ..default() }, BackgroundColor(ui::TEXT)));
            });
            meter.spawn((HandText, Text::new(""), TextFont { font_size: 12.0, ..default() }, TextColor(ui::TEXT), TextShadow::default()));
        });

    // Terrorist hunt status, under the score bar.
    commands.spawn((
        MatchEntity,
        HuntText,
        Text::new(""),
        TextFont { font_size: 15.0, ..default() },
        TextColor(ui::TEXT),
        TextShadow::default(),
        TextLayout::new_with_justify(JustifyText::Center),
        Node { position_type: PositionType::Absolute, top: Val::Px(50.0), width: Val::Percent(100.0), justify_content: JustifyContent::Center, ..default() },
        GlobalZIndex(-38),
    ));
}

#[allow(clippy::too_many_arguments)]
fn update(
    controls: Res<Controls>,
    settings: Res<Settings>,
    screen: Res<Screen>,
    rules: Res<Rules>,
    state: Res<MatchState>,
    hands: Res<Hands>,
    steady: Res<Steady>,
    view: Res<crate::view::ViewState>,
    (map, statics): (Res<Map>, Res<Statics>),
    players: Query<(&Player, &Transform, &PlayerInput, &Health, &Mover), With<LocalPlayer>>,
    mut panel: Query<&mut Visibility, (With<StreakPanel>, Without<HandMeter>)>,
    mut slots: Query<(&StreakSlot, &Children, &mut BackgroundColor, &mut BorderColor)>,
    mut texts: Query<&mut Text, (Without<StreakText>, Without<HandText>, Without<HuntText>)>,
    mut streak_text: Query<&mut Text, (With<StreakText>, Without<HandText>, Without<HuntText>)>,
    (mut actions, mut ledge): (ResMut<crate::hud::HandActions>, ResMut<Ledge>),
    (mut meter, mut fill, mut hand_text, mut hunt_text): (
        Query<&mut Visibility, (With<HandMeter>, Without<StreakPanel>)>,
        Query<(&mut Node, &mut BackgroundColor), (With<HandFill>, Without<StreakSlot>)>,
        Query<&mut Text, (With<HandText>, Without<StreakText>, Without<HuntText>)>,
        Query<&mut Text, (With<HuntText>, Without<StreakText>, Without<HandText>)>,
    ),
) {
    let Ok((player, transform, input, health, mover)) = players.single() else {
        return;
    };
    let playing = *screen == Screen::None && health.alive();
    let key = |action: Action| settings.bindings.label(action);
    let set = |text: &mut Text, value: String| {
        if text.0 != value {
            text.0 = value;
        }
    };

    // Killstreaks.
    let show_streaks = rules.pvp && playing && state.phase == Phase::Live;
    if let Ok(mut visibility) = panel.single_mut() {
        let wanted = if show_streaks { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    if show_streaks {
        // Newest on top: that is the one the button calls in.
        let stack: Vec<u8> = player.rewards.iter().rev().copied().filter(|r| *r != 0).collect();
        let call = if controls.gamepad { "D-PAD >".to_string() } else { key(Action::Killstreak) };
        for (slot, children, mut background, mut border) in &mut slots {
            let entry = stack.get(slot.0).copied();
            let line = match entry {
                Some(id) if slot.0 == 0 => format!("[{call}]  {}", killstreak::name(id)),
                Some(id) => killstreak::name(id).to_string(),
                None => "-".to_string(),
            };
            if let Some(mut text) = children.first().and_then(|c| texts.get_mut(*c).ok()) {
                set(&mut text, line);
            }
            let (fill, edge) = match entry {
                Some(_) if slot.0 == 0 => (Color::srgba(0.35, 0.28, 0.08, 0.9), Color::srgb(1.0, 0.8, 0.35)),
                Some(_) => (Color::srgba(0.06, 0.1, 0.13, 0.85), ui::ACCENT),
                None => (Color::srgba(0.02, 0.04, 0.06, 0.5), Color::srgba(0.3, 0.4, 0.42, 0.4)),
            };
            if background.0 != fill {
                background.0 = fill;
                border.0 = edge;
            }
        }
        let next = player.streaks.iter().filter_map(|id| killstreak::def(*id)).find(|d| d.kills > player.streak);
        let progress = match next {
            Some(def) => format!("STREAK {}   NEXT: {} IN {}", player.streak, def.name, def.kills - player.streak),
            None => format!("STREAK {}", player.streak),
        };
        if let Ok(mut text) = streak_text.single_mut() {
            set(&mut text, progress);
        }
    }

    // The meter under the crosshair: just the gauge and a word. What the buttons do goes in
    // the strip bottom left, and a vault shows as a chevron over the ledge itself.
    let pin_key = if controls.gamepad { "X".to_string() } else { key(Action::Reload) };
    let mut offered: Vec<(String, String)> = Vec::new();
    let mut over = None;
    let (shown, fraction, color, line) = if let Some(cooked) = hands.cooking.filter(|_| playing) {
        let left = (hands.fuse - cooked).max(0.0);
        let fraction = if hands.live { left / hands.fuse.max(0.01) } else { 1.0 };
        let color = if hands.live && left < 0.6 { ui::WARN } else { Color::srgb(1.0, 0.8, 0.35) };
        let gear_key = if controls.gamepad { "RB".to_string() } else { key(Action::Gear) };
        offered.push((gear_key, "RELEASE TO THROW".into()));
        offered.push((pin_key, if hands.live { "PIN BACK".into() } else { "PUT AWAY".into() }));
        (true, fraction, color, if hands.live { format!("{left:.1}") } else { String::new() })
    } else if view.scoped > 0.5 && playing {
        let stab = if controls.gamepad { "L3".to_string() } else { key(Action::Sprint) };
        if steady.left > 0.0 {
            (true, steady.left / steady.total.max(0.01), ui::GOOD, "STEADY".to_string())
        } else if steady.cooldown > 0.0 {
            (true, 1.0 - steady.cooldown / 2.0, ui::DIM, String::new())
        } else {
            offered.push((stab, format!("HOLD BREATH  {:.0}s", Steady::duration(input.stance))));
            (false, 1.0, ui::ACCENT, String::new())
        }
    } else {
        if playing && mover.grounded && input.stance != stance::PRONE && mover.vault.is_none() {
            // Something low ahead to go over?
            let wish = Quat::from_rotation_y(input.yaw) * Vec3::new(controls.movement.x, 0.0, -controls.movement.y);
            let dir = if wish.length() > 0.2 { wish } else { Quat::from_rotation_y(input.yaw) * Vec3::NEG_Z };
            let flat = Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero();
            if let Some((_, top)) = oz_sim::player::vault_target(&map, &statics, transform.translation, dir)
                && let Some((d, _)) = statics.raycast(transform.translation + Vec3::Y * 0.35, flat, 0.9)
            {
                let at = transform.translation + flat * (d + 0.1);
                over = Some(Vec3::new(at.x, top, at.z));
                offered.push((if controls.gamepad { "A".to_string() } else { key(Action::Jump) }, "VAULT".into()));
            }
        }
        (false, 0.0, ui::TEXT, String::new())
    };
    if actions.0 != offered {
        actions.0 = offered;
    }
    ledge.0 = over;
    if let Ok(mut visibility) = meter.single_mut() {
        let wanted = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    if let Ok((mut node, mut background)) = fill.single_mut() {
        let width = Val::Percent(fraction.clamp(0.0, 1.0) * 100.0);
        if node.width != width {
            node.width = width;
        }
        if background.0 != color {
            background.0 = color;
        }
    }
    if let Ok(mut text) = hand_text.single_mut() {
        set(&mut text, line);
    }

    // Terrorist hunt status.
    if let Ok(mut text) = hunt_text.single_mut() {
        let line = if rules.hunt && matches!(state.phase, Phase::Live) { format!("HOSTILES  {}      SQUAD LIVES  {}", state.hostiles, state.lives) } else { String::new() };
        set(&mut text, line);
    }
}

/// Pixels per metre in the baked map image.
const MAP_RES: f32 = 2.0;

/// Draws the map from above once per match: building footprints, walls and tall solids,
/// low cover, team bases and ladders.
fn bake_minimap(map: &oz_core::worldgen::World) -> (Image, f32) {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let extent = map.recipe.half_extent.min(110.0);
    let size = (extent * 2.0 * MAP_RES) as usize;
    let mut pixels = vec![0u8; size * size * 4];
    let mut fill = |x0: f32, z0: f32, x1: f32, z1: f32, color: [u8; 4]| {
        let to = |v: f32| (((v + extent) * MAP_RES).floor().max(0.0) as usize).min(size - 1);
        for v in to(z0)..=to(z1) {
            for u in to(x0)..=to(x1) {
                let i = (v * size + u) * 4;
                pixels[i..i + 4].copy_from_slice(&color);
            }
        }
    };
    // Floors first (foundations and slabs), then low cover, then anything tall on top.
    let mut layers: [Vec<(f32, f32, f32, f32)>; 3] = Default::default();
    for block in map.blocks.iter().filter(|b| b.solid) {
        let b = block.aabb;
        let ground = map.height(b.center.x, b.center.z);
        let rise = b.top() - ground;
        let rect = (b.center.x - b.half.x, b.center.z - b.half.z, b.center.x + b.half.x, b.center.z + b.half.z);
        if b.half.x > 1.5 && b.half.z > 1.5 && rise < 0.5 {
            layers[0].push(rect);
        } else if rise > 1.6 {
            layers[2].push(rect);
        } else if rise > 0.5 {
            layers[1].push(rect);
        }
    }
    for prop in &map.props {
        if let Some(half) = prop.solid_half() {
            layers[if half.y * 2.0 > 1.6 { 2 } else { 1 }].push((prop.pos.x - half.x, prop.pos.z - half.z, prop.pos.x + half.x, prop.pos.z + half.z));
        }
    }
    for clutter in &map.clutter {
        let (c, h) = (clutter.pos, clutter.half());
        layers[1].push((c.x - h.x, c.z - h.z, c.x + h.x, c.z + h.z));
    }
    for (layer, color) in layers.iter().zip([[40, 54, 60, 235], [92, 108, 98, 230], [150, 172, 172, 255]]) {
        for &(x0, z0, x1, z1) in layer {
            fill(x0, z0, x1, z1, color);
        }
    }
    // Points of interest: team bases and the ladders up to the roofs.
    for (team, color) in [(0usize, [64u8, 204, 209, 255]), (1, [242, 133, 107, 255])] {
        for p in &map.team_spawns[team] {
            fill(p.x - 1.2, p.y - 1.2, p.x + 1.2, p.y + 1.2, color);
        }
    }
    for ladder in &map.ladders {
        fill(ladder.foot.x - 0.6, ladder.foot.z - 0.6, ladder.foot.x + 0.6, ladder.foot.z + 0.6, [255, 204, 102, 255]);
    }
    let image = Image::new(
        Extent3d { width: size as u32, height: size as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    (image, extent)
}

/// Keeps the radar's map under the blips (north up, centred on you) and turns your arrow.
fn minimap(
    mut commands: Commands,
    map: Res<Map>,
    mut images: ResMut<Assets<Image>>,
    players: Query<(&Transform, &PlayerInput), (With<LocalPlayer>, Without<MinimapPivot>, Without<RadarArrow>)>,
    pivots: Query<(Entity, Option<&Children>), With<MinimapPivot>>,
    mut arrows: Query<&mut Transform, (With<RadarArrow>, Without<LocalPlayer>)>,
    mut layers: Query<(&MinimapImage, &mut Node)>,
) {
    let Ok((pivot, children)) = pivots.single() else { return };
    let Ok((me, input)) = players.single() else { return };
    let scale = (crate::hud::RADAR / 2.0 - 4.0) / crate::hud::RADAR_RANGE;
    if children.is_none_or(|c| c.is_empty()) {
        let (image, extent) = bake_minimap(&map.0);
        let handle = images.add(image);
        let side = extent * 2.0 * scale;
        commands.spawn((
            MinimapImage { extent },
            ImageNode::new(handle),
            Node { position_type: PositionType::Absolute, width: Val::Px(side), height: Val::Px(side), ..default() },
            ChildOf(pivot),
        ));
        return;
    }
    for mut arrow in &mut arrows {
        arrow.rotation = Quat::from_rotation_z(-input.yaw);
    }
    for (layer, mut node) in &mut layers {
        let left = Val::Px(-(me.translation.x + layer.extent) * scale);
        let top = Val::Px(-(me.translation.z + layer.extent) * scale);
        if node.left != left || node.top != top {
            node.left = left;
            node.top = top;
        }
    }
}

/// Turns a loot pickup's model slowly (the pickup itself is placed by the network).
#[derive(Component)]
pub struct LootSpin;

/// Models for scavenge loot: the real gun with its parts, or a box, plates or a crate.
fn dress_loot(
    mut commands: Commands,
    art: Res<crate::art::Art>,
    content: Res<Content>,
    mut guns: ResMut<crate::avatar::GunCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    added: Query<(Entity, &oz_sim::loot::Pickup), Added<oz_sim::loot::Pickup>>,
) {
    use oz_sim::loot::Loot;
    for (entity, pickup) in &added {
        let baked = match pickup.loot {
            Loot::Weapon { index, parts, .. } => guns.get(&art, &mut meshes, &content, index, parts),
            Loot::Ammo => Some(art.bake(&BlockModel::new().cube(Paint::Green, [0.0, 0.18, 0.0], [0.55, 0.32, 0.32]).cube(Paint::Gold, [0.0, 0.36, 0.0], [0.3, 0.05, 0.12]).cube(Paint::Dark, [0.0, 0.18, -0.165], [0.4, 0.08, 0.01]), &mut meshes)),
            Loot::Armor => Some(art.bake(&BlockModel::new().cube(Paint::Steel, [0.0, 0.32, 0.0], [0.5, 0.6, 0.12]).cube(Paint::Dark, [0.0, 0.32, -0.07], [0.36, 0.42, 0.02]).cube(Paint::Aether, [0.0, 0.5, -0.085], [0.2, 0.05, 0.01]), &mut meshes)),
            Loot::Gear(_) => Some(art.bake(&BlockModel::new().cube(Paint::Rust, [0.0, 0.15, 0.0], [0.45, 0.3, 0.45]).cylinder(Paint::Iron, [0.0, 0.38, 0.0], [0.14, 0.18, 0.14]).cube(Paint::Gold, [0.0, 0.15, -0.23], [0.3, 0.06, 0.01]), &mut meshes)),
            Loot::Supply { .. } => {
                // A banded wooden crate with a medical cross, and a parachute over it while it falls.
                let body = BlockModel::new()
                    .cube(Paint::Wood, [0.0, 0.42, 0.0], [1.0, 0.84, 0.8])
                    .cube(Paint::Iron, [0.0, 0.2, 0.0], [1.02, 0.07, 0.82])
                    .cube(Paint::Iron, [0.0, 0.66, 0.0], [1.02, 0.07, 0.82])
                    .cube(Paint::Bone, [0.0, 0.42, -0.406], [0.34, 0.1, 0.01])
                    .cube(Paint::Bone, [0.0, 0.42, -0.406], [0.1, 0.34, 0.01])
                    .cube(Paint::Bone, [0.506, 0.42, 0.0], [0.01, 0.1, 0.34])
                    .cube(Paint::Bone, [0.506, 0.42, 0.0], [0.01, 0.34, 0.1])
                    .shard(Paint::Green, [0.0, 0.9, 0.0], [0.08, 0.08, 0.08]);
                let chute = BlockModel::new()
                    .cone(Paint::Coral, [0.0, 4.2, 0.0], [3.4, 1.3, 3.4])
                    .cylinder(Paint::Bone, [0.0, 3.62, 0.0], [3.3, 0.06, 3.3])
                    .segment(Paint::Bone, [0.45, 0.84, 0.35], [1.55, 3.6, 1.2], 0.02)
                    .segment(Paint::Bone, [-0.45, 0.84, 0.35], [-1.55, 3.6, 1.2], 0.02)
                    .segment(Paint::Bone, [0.45, 0.84, -0.35], [1.55, 3.6, -1.2], 0.02)
                    .segment(Paint::Bone, [-0.45, 0.84, -0.35], [-1.55, 3.6, -1.2], 0.02);
                commands.entity(entity).insert(Visibility::Inherited);
                let crate_root = commands.spawn((Transform::default(), Visibility::Inherited, ChildOf(entity))).id();
                art.spawn(&mut commands, crate_root, &art.bake(&body, &mut meshes));
                let canopy = commands.spawn((Parachute, Transform::default(), Visibility::Inherited, ChildOf(entity))).id();
                art.spawn(&mut commands, canopy, &art.bake(&chute, &mut meshes));
                continue;
            }
        };
        commands.entity(entity).insert(Visibility::Inherited);
        let lift = if matches!(pickup.loot, Loot::Weapon { .. }) { 0.55 } else { 0.05 };
        let spin = commands.spawn((LootSpin, Transform::from_xyz(0.0, lift, 0.0), Visibility::Inherited, ChildOf(entity))).id();
        if let Some(baked) = baked {
            art.spawn(&mut commands, spin, &baked);
        }
        // A glowing ring on the ground so loot reads at a glance.
        commands.spawn((
            Mesh3d(art.primitive(Shape::Cylinder)),
            MeshMaterial3d(art.paint(if matches!(pickup.loot, Loot::Weapon { .. }) { Paint::Glow } else { Paint::Aether })),
            Transform::from_xyz(0.0, 0.02, 0.0).with_scale(Vec3::new(0.9, 0.02, 0.9)),
            bevy::pbr::NotShadowCaster,
            ChildOf(entity),
        ));
    }
}

/// A falling supply crate's canopy: swaying on the way down, packed away once it lands.
#[derive(Component)]
struct Parachute;

fn sway_chutes(time: Res<Time>, map: Res<Map>, statics: Res<oz_sim::session::Statics>, crates: Query<(&Transform, &Children), (With<oz_sim::loot::Pickup>, Without<Parachute>)>, mut chutes: Query<(&mut Transform, &mut Visibility), With<Parachute>>) {
    let t = time.elapsed_secs();
    for (at, children) in &crates {
        let p = at.translation;
        let ground = map.height(p.x, p.z).max(statics.support(p.x, p.z, p.y + 0.3));
        let falling = p.y - ground > 0.15;
        for child in children.iter() {
            if let Ok((mut transform, mut visibility)) = chutes.get_mut(child) {
                let wanted = if falling { Visibility::Inherited } else { Visibility::Hidden };
                if *visibility != wanted {
                    *visibility = wanted;
                }
                transform.rotation = Quat::from_rotation_z((t * 1.3).sin() * 0.08) * Quat::from_rotation_x((t * 0.9).cos() * 0.06);
            }
        }
    }
}

fn spin_loot(time: Res<Time>, mut spins: Query<&mut Transform, With<LootSpin>>) {
    let t = time.elapsed_secs();
    for mut transform in &mut spins {
        transform.rotation = Quat::from_rotation_y(t * 1.2);
    }
}

/// Scavenge: near a gun with both hands full, Use swaps for it.
fn loot_prompt(
    controls: Res<Controls>,
    settings: Res<Settings>,
    screen: Res<Screen>,
    content: Res<Content>,
    rules: Res<Rules>,
    players: Query<(&Player, &Transform, &Arsenal, &Health), With<LocalPlayer>>,
    pickups: Query<(&Transform, &oz_sim::loot::Pickup)>,
    mut orders: EventWriter<CommandEvent>,
    mut actions: ResMut<crate::hud::HandActions>,
) {
    use oz_sim::loot::Loot;
    let Ok((me, transform, arsenal, health)) = players.single() else { return };
    if !rules.loot || !health.alive() || *screen != Screen::None {
        return;
    }
    let owned = (0..arsenal.weapons.len()).filter(|i| arsenal.owns(*i)).count();
    let near = pickups
        .iter()
        .filter(|(at, _)| (at.translation - transform.translation).xz().length() < 1.4 && (at.translation.y - transform.translation.y).abs() < 1.6)
        .find_map(|(_, p)| match p.loot {
            Loot::Weapon { index, .. } if owned >= 2 && !arsenal.owns(index as usize) => content.weapons.get(index as usize).map(|d| d.name),
            _ => None,
        });
    let Some(name) = near else { return };
    if controls.interact {
        orders.write(CommandEvent { player: me.id, command: PlayerCommand::Pickup });
    }
    let key = if controls.gamepad { "X".to_string() } else { settings.bindings.label(Action::Interact) };
    actions.0.insert(0, (key, format!("SWAP FOR {name}")));
}

/// A flat up-pointing chevron in the XY plane (seen from both sides): `width` across the
/// tips, `height` from tips to point, arms `thick` deep.
fn chevron_mesh(width: f32, height: f32, thick: f32) -> Mesh {
    use bevy::render::{mesh::Indices, render_asset::RenderAssetUsages, render_resource::PrimitiveTopology};
    let half = width / 2.0;
    let inner = half * (height - thick) / height;
    let base = -height / 2.0;
    let outline = [[0.0, height / 2.0], [half, base], [inner, base], [0.0, height / 2.0 - thick], [-inner, base], [-half, base]];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for normal in [Vec3::Z, Vec3::NEG_Z] {
        for [x, y] in outline {
            positions.push([x, y, 0.0]);
            normals.push(normal.to_array());
        }
    }
    // A fan from the point, once facing each way.
    let mut indices = Vec::new();
    for i in 1..5u32 {
        indices.extend([0, i + 1, i]);
        indices.extend([6, 6 + i, 6 + i + 1]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_indices(Indices::U32(indices))
}

/// A glowing chevron bobbing over the ledge ahead: "you can go over this".
fn point_at_ledge(
    mut commands: Commands,
    time: Res<Time>,
    art: Res<crate::art::Art>,
    ledge: Res<Ledge>,
    mut meshes: ResMut<Assets<Mesh>>,
    camera: Query<&GlobalTransform, With<crate::view::WorldCamera>>,
    mut arrow: Query<(&mut Transform, &mut Visibility), With<LedgeArrow>>,
) {
    let Ok((mut transform, mut visibility)) = arrow.single_mut() else {
        // One flat chevron, pointing up, with a clean sharp tip.
        commands.spawn((
            MatchEntity,
            LedgeArrow,
            Mesh3d(meshes.add(chevron_mesh(0.13, 0.075, 0.03))),
            MeshMaterial3d(art.paint(Paint::Glow)),
            Transform::default(),
            Visibility::Hidden,
            bevy::pbr::NotShadowCaster,
        ));
        return;
    };
    let wanted = if ledge.0.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    if *visibility != wanted {
        *visibility = wanted;
    }
    let Some(at) = ledge.0 else { return };
    let bob = (time.elapsed_secs() * 4.0).sin() * 0.05;
    transform.translation = at + Vec3::Y * (0.3 + bob);
    // Always faces the camera.
    if let Ok(eye) = camera.single() {
        let to = eye.translation() - transform.translation;
        transform.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
    }
}

/// The helmet flashlight: on by default (it only matters in the dark).
#[derive(Resource, Clone, Copy, Debug)]
pub struct Lamp {
    pub on: bool,
}

impl Default for Lamp {
    fn default() -> Self {
        Self { on: true }
    }
}

/// The lamp's state rides your input to the host, so everyone else sees your beam (and
/// the bots notice it).
fn share_lamp(lamp: Res<Lamp>, mut players: Query<(&mut PlayerInput, &Health), With<LocalPlayer>>) {
    for (mut input, health) in &mut players {
        let on = lamp.on && health.alive();
        if input.light != on {
            input.light = on;
        }
    }
}

/// D-pad down, Rainbow-style: tap to screw the suppressor on or off (if the gun came with
/// one), hold to switch the flashlight. U and H on a keyboard.
#[allow(clippy::too_many_arguments)]
fn toggles(
    controls: Res<Controls>,
    screen: Res<Screen>,
    content: Res<Content>,
    players: Query<(&Player, &Arsenal, &PlayerInput, &Health), With<LocalPlayer>>,
    mut lamp: ResMut<Lamp>,
    mut orders: EventWriter<CommandEvent>,
    mut notices: ResMut<crate::hud::Notices>,
    mut sounds: EventWriter<crate::fx::PlaySfx>,
    mut unscrewed: Local<std::collections::HashSet<u8>>,
) {
    use oz_core::arsenal::slot;
    let Ok((me, arsenal, input, health)) = players.single() else { return };
    if *screen != Screen::None || !health.alive() {
        return;
    }
    if controls.flashlight {
        lamp.on = !lamp.on;
        notices.push(if lamp.on { "FLASHLIGHT ON".into() } else { "FLASHLIGHT OFF".into() });
        sounds.write(crate::fx::PlaySfx::ui(Sfx::UiMove));
    }
    if !controls.suppressor {
        return;
    }
    let weapon = crate::view::held_weapon(input, arsenal);
    let (Some(def), Some(state)) = (content.weapons.get(weapon as usize), arsenal.weapons.get(weapon as usize)) else { return };
    let choices = options(def, &content.attachments, slot::MUZZLE);
    let Some(suppressor) = choices.iter().find(|(_, name, ..)| name.contains("SUPPRESS")).map(|(i, ..)| *i as u8) else {
        notices.push("THIS GUN TAKES NO SUPPRESSOR".into());
        return;
    };
    let fitted = state.equipped[slot::MUZZLE];
    let variant = if fitted == suppressor {
        // Off: back to the bare muzzle.
        unscrewed.insert(weapon);
        let bare = if def.defaults[slot::MUZZLE] != suppressor { def.defaults[slot::MUZZLE] } else { choices.iter().map(|(i, ..)| *i as u8).find(|i| *i != suppressor).unwrap_or(0) };
        notices.push("SUPPRESSOR OFF".into());
        bare
    } else if unscrewed.remove(&weapon) {
        notices.push("SUPPRESSOR ON".into());
        suppressor
    } else {
        notices.push("NO SUPPRESSOR FOR THIS GUN - FIT ONE IN THE ARMORY".into());
        return;
    };
    orders.write(CommandEvent { player: me.id, command: PlayerCommand::EquipPart { weapon, slot: slot::MUZZLE as u8, variant } });
    sounds.write(crate::fx::PlaySfx::new(Sfx::Bolt, None).quiet(0.7));
}
