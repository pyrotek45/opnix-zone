//! Objective modes on screen (see `oz_sim::objective`).
//!
//! * Domination: the A / B / C flags under the score bar in their holders' colours, with
//!   a capture gauge; a ring and a flag pole at each point in the world.
//! * Search and destroy: the round clock, who is still standing on each side, what the
//!   bomb is doing; the bomb itself in the world, ticking once planted.
//! * Both: markers over every point (letter, colour and distance, seen through walls and
//!   pinned to the screen edge when off screen), icons on the radar, a plant / defuse /
//!   capture gauge under the crosshair and the key to hold.

use crate::{
    ClientSet,
    bindings::Action,
    input::Controls,
    lobby::TEAM_COLORS,
    settings::Settings,
    ui::{self, Screen},
    view::WorldCamera,
};
use bevy::prelude::*;
use oz_sim::{
    objective::{DEFUSE_REACH, LETTERS, NEUTRAL, POINT_RADIUS, SITE_RADIUS, bomb, in_site, kind, stage},
    prelude::*,
};

/// World markers: up to three points plus the bomb.
const MARKERS: u8 = 4;
const PIPS: u8 = 6;
const NEUTRAL_COLOR: Color = Color::srgb(0.82, 0.84, 0.86);
const OFF: Color = Color::srgba(0.10, 0.14, 0.17, 0.75);
const BOMB_COLOR: Color = Color::srgb(1.0, 0.62, 0.2);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
enum Part {
    /// Domination's row of flags.
    Flags,
    Flag(u8),
    FlagFill(u8),
    FlagLetter(u8),
    /// Search and destroy's round panel.
    Round,
    Pip(u8, u8),
    Clock,
    Status,
    Meter,
    MeterFill,
    MeterText,
    Prompt,
    Marker(u8),
    MarkerBox(u8),
    MarkerLetter(u8),
    MarkerText(u8),
    Mini(u8),
    MiniLetter(u8),
}

/// 3D furniture: a ring and a flag on each point, and the bomb.
#[derive(Component)]
struct PointProp;

#[derive(Component)]
struct BombProp;

#[derive(Resource, Default)]
struct Props {
    spawned: Vec<Vec3>,
    paints: Vec<Handle<StandardMaterial>>,
    bomb: Option<Handle<StandardMaterial>>,
}

pub struct ObjectivesPlugin;

impl Plugin for ObjectivesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Props>()
            .add_systems(OnEnter(MatchPhase::Active), (spawn, |mut props: ResMut<Props>| *props = Props::default()))
            .add_systems(Update, (panels, markers.after(crate::spectate::Spectating), minimap_icons, props).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

fn text(size: f32, color: Color) -> impl Bundle {
    (Text::new(""), TextFont { font_size: size, ..default() }, TextColor(color), TextShadow::default(), TextLayout::new_with_justify(JustifyText::Center))
}

fn spawn(mut commands: Commands) {
    let across = |top: Val| Node { position_type: PositionType::Absolute, top, width: Val::Percent(100.0), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, row_gap: Val::Px(4.0), ..default() };
    // Domination: A B C under the scores.
    commands.spawn((MatchEntity, Part::Flags, across(Val::Px(48.0)), GlobalZIndex(-39), Visibility::Hidden)).with_children(|top| {
        top.spawn(Node { column_gap: Val::Px(10.0), ..default() }).with_children(|row| {
            for i in 0..3u8 {
                row.spawn((
                    Part::Flag(i),
                    Node { width: Val::Px(36.0), height: Val::Px(36.0), border: UiRect::all(Val::Px(2.0)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
                    BackgroundColor(OFF),
                    BorderColor(NEUTRAL_COLOR),
                ))
                .with_children(|flag| {
                    flag.spawn((Part::FlagLetter(i), text(19.0, Color::WHITE)));
                    flag.spawn((Part::FlagFill(i), Node { position_type: PositionType::Absolute, left: Val::Px(0.0), bottom: Val::Px(0.0), height: Val::Px(5.0), width: Val::Percent(0.0), ..default() }, BackgroundColor(Color::WHITE)));
                });
            }
        });
    });
    // Search and destroy: who is up, the clock, the bomb.
    commands.spawn((MatchEntity, Part::Round, across(Val::Px(48.0)), GlobalZIndex(-39), Visibility::Hidden)).with_children(|top| {
        top.spawn(Node { column_gap: Val::Px(14.0), align_items: AlignItems::Center, ..default() }).with_children(|row| {
            for team in 0..2u8 {
                if team == 1 {
                    row.spawn((Part::Clock, text(17.0, ui::TEXT)));
                }
                row.spawn(Node { column_gap: Val::Px(3.0), flex_direction: if team == 0 { FlexDirection::RowReverse } else { FlexDirection::Row }, ..default() }).with_children(|pips| {
                    for i in 0..PIPS {
                        pips.spawn((Part::Pip(team, i), Node { width: Val::Px(9.0), height: Val::Px(14.0), ..default() }, BackgroundColor(OFF)));
                    }
                });
            }
        });
        top.spawn((Part::Status, text(14.0, ui::TEXT)));
    });
    // The gauge under the crosshair, and what to hold.
    commands.spawn((MatchEntity, Part::Meter, across(Val::Percent(60.0)), GlobalZIndex(-38), Visibility::Hidden)).with_children(|meter| {
        meter.spawn((Part::MeterText, text(14.0, ui::TEXT)));
        meter.spawn((Node { width: Val::Px(220.0), height: Val::Px(8.0), border: UiRect::all(Val::Px(1.0)), ..default() }, BackgroundColor(OFF), BorderColor(Color::srgba(0.8, 0.85, 0.85, 0.6)))).with_children(|track| {
            track.spawn((Part::MeterFill, Node { height: Val::Percent(100.0), width: Val::Percent(0.0), ..default() }, BackgroundColor(ui::TEXT)));
        });
    });
    commands.spawn((MatchEntity, Part::Prompt, text(17.0, ui::ACCENT), Node { position_type: PositionType::Absolute, top: Val::Percent(66.0), width: Val::Percent(100.0), justify_content: JustifyContent::Center, ..default() }, GlobalZIndex(-38)));
    // Markers over the points, seen through walls.
    for i in 0..MARKERS {
        commands
            .spawn((
                MatchEntity,
                Part::Marker(i),
                Node { position_type: PositionType::Absolute, width: Val::Px(80.0), margin: UiRect::new(Val::Px(-40.0), Val::ZERO, Val::Px(-14.0), Val::ZERO), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, ..default() },
                Visibility::Hidden,
                GlobalZIndex(-43),
            ))
            .with_children(|marker| {
                marker
                    .spawn((
                        Part::MarkerBox(i),
                        Node { min_width: Val::Px(22.0), height: Val::Px(22.0), padding: UiRect::horizontal(Val::Px(3.0)), border: UiRect::all(Val::Px(2.0)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
                        BackgroundColor(Color::srgba(0.02, 0.04, 0.06, 0.55)),
                        BorderColor(NEUTRAL_COLOR),
                    ))
                    .with_children(|letter| {
                        letter.spawn((Part::MarkerLetter(i), text(14.0, Color::WHITE)));
                    });
                marker.spawn((Part::MarkerText(i), text(11.0, ui::TEXT)));
            });
    }
}

fn team_color(team: u8) -> Color {
    if team < 2 { TEAM_COLORS[team as usize] } else { NEUTRAL_COLOR }
}

fn mmss(seconds: f32) -> String {
    let s = seconds.max(0.0).ceil() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

fn set<T: PartialEq>(slot: &mut T, value: T) {
    if *slot != value {
        *slot = value;
    }
}

/// A marker: where, the letter, its colour, and the line under it.
struct Mark {
    at: Vec3,
    letter: String,
    color: Color,
    line: String,
}

/// What the objective looks like from where the local player stands.
fn marks(state: &MatchState, me: &Player, feet: Vec3, now: f32) -> Vec<Mark> {
    let obj = &state.obj;
    let mut out = Vec::new();
    let blink = (now * 4.0).fract() < 0.5;
    let distance = |at: Vec3| format!("{:.0}m", at.distance(feet));
    match obj.kind {
        kind::DOMINATION => {
            for (i, point) in obj.points.iter().enumerate() {
                let color = if point.contested && blink { ui::WARN } else { team_color(point.owner) };
                out.push(Mark { at: point.at + Vec3::Y * 3.4, letter: LETTERS[i.min(2)].into(), color, line: distance(point.at) });
            }
        }
        kind::BOMB if !obj.points.is_empty() => {
            let attacking = me.team == obj.attackers;
            for (i, site) in obj.points.iter().enumerate() {
                let planted_here = obj.bomb == bomb::PLANTED && obj.site as usize == i;
                let (color, word) = if planted_here {
                    (if blink { ui::WARN } else { BOMB_COLOR }, if attacking { "GUARD" } else { "DEFUSE" })
                } else if attacking {
                    (BOMB_COLOR, "PLANT")
                } else {
                    (team_color(me.team), "DEFEND")
                };
                out.push(Mark { at: site.at + Vec3::Y * 2.6, letter: LETTERS[i.min(2)].into(), color, line: format!("{word}  {}", distance(site.at)) });
            }
            // The bomb: attackers always know where it is; everyone sees a planted one.
            let show = match obj.bomb {
                bomb::HOME | bomb::DROPPED => attacking,
                bomb::CARRIED => attacking && obj.carrier != me.id,
                bomb::PLANTED => true,
                _ => false,
            };
            if show {
                let lift = if obj.bomb == bomb::CARRIED { 2.4 } else { 0.8 };
                let line = if obj.bomb == bomb::PLANTED { mmss(obj.fuse) } else { distance(obj.bomb_at) };
                out.push(Mark { at: obj.bomb_at + Vec3::Y * lift, letter: "BOMB".into(), color: BOMB_COLOR, line });
            }
        }
        _ => {}
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn panels(
    time: Res<Time>,
    state: Res<MatchState>,
    rules: Res<Rules>,
    screen: Res<Screen>,
    controls: Res<Controls>,
    settings: Res<Settings>,
    local: Query<(&Player, &Transform, &Health), With<LocalPlayer>>,
    mut nodes: Query<(&Part, &mut Node, Option<&mut BackgroundColor>, Option<&mut BorderColor>, Option<&mut Visibility>), Without<Text>>,
    mut texts: Query<(&Part, &mut Text, &mut TextColor)>,
) {
    let obj = &state.obj;
    let Ok((me, transform, health)) = local.single() else { return };
    let feet = transform.translation;
    let shown = matches!(*screen, Screen::None | Screen::Chat) && rules.pvp;
    let now = time.elapsed_secs();
    let blink = (now * 4.0).fract() < 0.5;
    let dom = shown && obj.kind == kind::DOMINATION && !obj.points.is_empty() && matches!(state.phase, Phase::Lobby | Phase::Live | Phase::Won);
    let round = shown && obj.kind == kind::BOMB && obj.round > 0 && matches!(state.phase, Phase::Live | Phase::Won);
    let key = if controls.gamepad { "X".to_string() } else { settings.bindings.label(Action::Interact) };
    let attacking = me.team == obj.attackers;
    let alive = health.alive() && state.phase == Phase::Live;

    // The gauge: planting or defusing, or taking (or losing) the point you stand in.
    let mut meter: Option<(String, f32, Color)> = None;
    let mut prompt = String::new();
    if alive && obj.kind == kind::BOMB && obj.stage == stage::PLAYING {
        if obj.actor == me.id && obj.progress > 0.0 {
            let word = if obj.bomb == bomb::PLANTED { "DEFUSING" } else { "PLANTING" };
            meter = Some((word.into(), obj.progress, if obj.bomb == bomb::PLANTED { team_color(me.team) } else { BOMB_COLOR }));
        } else if attacking && obj.bomb == bomb::CARRIED && obj.carrier == me.id && obj.points.iter().any(|s| in_site(s.at, feet)) {
            prompt = format!("HOLD [{key}] TO PLANT THE BOMB");
        } else if !attacking && obj.bomb == bomb::PLANTED && obj.bomb_at.distance(feet) < DEFUSE_REACH {
            prompt = format!("HOLD [{key}] TO DEFUSE");
        }
    }
    if alive
        && obj.kind == kind::DOMINATION
        && let Some((i, point)) = obj.points.iter().enumerate().find(|(_, p)| p.holds(feet))
    {
        let letter = LETTERS[i.min(2)];
        if point.contested {
            meter = Some((format!("{letter} CONTESTED"), point.progress, ui::WARN));
        } else if point.taking != NEUTRAL && point.progress > 0.0 {
            let word = if point.taking == me.team { "TAKING" } else { "LOSING" };
            meter = Some((format!("{word} {letter}"), point.progress, team_color(point.taking)));
        } else if point.owner == me.team {
            meter = Some((format!("HOLDING {letter}"), 1.0, team_color(me.team)));
        }
    }
    let meter = meter.filter(|_| shown);

    // Search and destroy's lines.
    let (clock, clock_color, status, status_color) = if round {
        let side = if attacking { "ATTACK" } else { "DEFEND" };
        let (clock, color) = if obj.bomb == bomb::PLANTED && obj.stage == stage::PLAYING {
            (format!("ROUND {}   BOMB {}", obj.round, mmss(obj.fuse)), if blink && obj.fuse < 10.0 { ui::WARN } else { BOMB_COLOR })
        } else if obj.stage == stage::OVER {
            (format!("ROUND {}   --:--", obj.round), ui::DIM)
        } else {
            (format!("ROUND {}   {}", obj.round, mmss(obj.clock)), if obj.clock < 15.0 { ui::WARN } else { ui::TEXT })
        };
        let site = LETTERS[(obj.site as usize).min(2)];
        let (status, tint) = if state.phase == Phase::Won {
            (String::new(), ui::TEXT)
        } else if obj.stage == stage::OVER {
            (format!("{}  -  NEXT ROUND IN {:.0}", obj.last_reason, obj.clock.max(0.0).ceil()), team_color(obj.last_winner))
        } else {
            match (attacking, obj.bomb) {
                (true, bomb::HOME) => (format!("{side}  -  PICK UP THE BOMB AT YOUR BASE"), ui::TEXT),
                (true, bomb::DROPPED) => (format!("{side}  -  THE BOMB IS DOWN: PICK IT UP"), BOMB_COLOR),
                (true, bomb::CARRIED) if obj.carrier == me.id => (format!("{side}  -  YOU HAVE THE BOMB: PLANT IT AT A OR B"), BOMB_COLOR),
                (true, bomb::CARRIED) => (format!("{side}  -  ESCORT THE BOMB CARRIER"), ui::TEXT),
                (true, _) => (format!("{side}  -  BOMB PLANTED AT {site}: GUARD IT"), BOMB_COLOR),
                (false, bomb::PLANTED) => (format!("{side}  -  BOMB PLANTED AT {site}: DEFUSE IT"), ui::WARN),
                (false, _) => (format!("{side}  -  PROTECT SITES A AND B"), ui::TEXT),
            }
        };
        (clock, color, status, tint)
    } else {
        (String::new(), ui::TEXT, String::new(), ui::TEXT)
    };
    let members = |team: u8| state.roster.iter().filter(|r| r.team == team).count() as u8;

    for (part, mut node, background, border, visibility) in &mut nodes {
        let visible = match part {
            Part::Flags => Some(dom),
            Part::Round => Some(round),
            Part::Meter => Some(meter.is_some()),
            _ => None,
        };
        if let (Some(on), Some(mut visibility)) = (visible, visibility) {
            set(&mut *visibility, if on { Visibility::Inherited } else { Visibility::Hidden });
        }
        match *part {
            Part::Flag(i) => {
                let Some(point) = obj.points.get(i as usize) else { continue };
                set(&mut node.display, if (i as usize) < obj.points.len() { Display::Flex } else { Display::None });
                let edge = if point.contested && blink { ui::WARN } else { team_color(point.owner) };
                let fill = if point.owner < 2 { team_color(point.owner).with_alpha(0.55) } else { OFF };
                if let Some(mut background) = background {
                    set(&mut background.0, fill);
                }
                if let Some(mut border) = border {
                    set(&mut border.0, edge);
                }
            }
            Part::FlagFill(i) => {
                let Some(point) = obj.points.get(i as usize) else { continue };
                set(&mut node.width, Val::Percent(point.progress.clamp(0.0, 1.0) * 100.0));
                if let Some(mut background) = background {
                    set(&mut background.0, team_color(point.taking));
                }
            }
            Part::Pip(team, i) => {
                let standing = obj.alive[team as usize];
                set(&mut node.display, if i < members(team).max(standing) { Display::Flex } else { Display::None });
                if let Some(mut background) = background {
                    set(&mut background.0, if i < standing { TEAM_COLORS[team as usize] } else { OFF });
                }
            }
            Part::MeterFill => {
                if let Some((_, fraction, color)) = &meter {
                    set(&mut node.width, Val::Percent(fraction.clamp(0.0, 1.0) * 100.0));
                    if let Some(mut background) = background {
                        set(&mut background.0, *color);
                    }
                }
            }
            _ => {}
        }
    }
    for (part, mut text, mut color) in &mut texts {
        let (wanted, tint) = match *part {
            Part::FlagLetter(i) => (LETTERS[(i as usize).min(2)].to_string(), Color::WHITE),
            Part::Clock => (clock.clone(), clock_color),
            Part::Status => (status.clone(), status_color),
            Part::MeterText => (meter.as_ref().map_or(String::new(), |m| m.0.clone()), ui::TEXT),
            Part::Prompt => (if shown { prompt.clone() } else { String::new() }, ui::ACCENT),
            _ => continue,
        };
        set(&mut text.0, wanted);
        set(&mut color.0, tint);
    }
}

/// Markers over the points, through walls; pinned to the screen edge when off screen.
#[allow(clippy::too_many_arguments)]
fn markers(
    time: Res<Time>,
    state: Res<MatchState>,
    screen: Res<Screen>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    local: Query<(&Player, &Transform), With<LocalPlayer>>,
    mut nodes: Query<(&Part, &mut Node, Option<&mut BorderColor>, Option<&mut Visibility>), Without<Text>>,
    mut texts: Query<(&Part, &mut Text, &mut TextColor)>,
) {
    let (Ok((camera, view)), Ok((me, transform))) = (camera.single(), local.single()) else { return };
    let showing = matches!(*screen, Screen::None | Screen::Chat) && matches!(state.phase, Phase::Live | Phase::Lobby);
    let list = if showing { marks(&state, me, transform.translation, time.elapsed_secs()) } else { Vec::new() };
    let forward = view.forward().as_vec3();
    let place = |at: Vec3| -> Option<Vec2> {
        let ndc = camera.world_to_ndc(view, at)?;
        let ahead = (at - view.translation()).dot(forward) > 0.0;
        let mut p = Vec2::new(ndc.x, ndc.y);
        if !ahead {
            // Behind you: push it to the edge on the side it is on.
            p = -p;
            p /= p.abs().max_element().max(1e-3);
        }
        Some(p.clamp(Vec2::splat(-0.93), Vec2::new(0.93, 0.86)))
    };
    for (part, mut node, border, visibility) in &mut nodes {
        match *part {
            Part::Marker(i) => {
                let spot = list.get(i as usize).and_then(|m| place(m.at));
                if let Some(mut visibility) = visibility {
                    set(&mut *visibility, if spot.is_some() { Visibility::Inherited } else { Visibility::Hidden });
                }
                if let Some(p) = spot {
                    set(&mut node.left, Val::Percent((p.x + 1.0) * 50.0));
                    set(&mut node.top, Val::Percent((1.0 - p.y) * 50.0));
                }
            }
            Part::MarkerBox(i) => {
                if let (Some(mark), Some(mut border)) = (list.get(i as usize), border) {
                    set(&mut border.0, mark.color);
                }
            }
            _ => {}
        }
    }
    for (part, mut text, mut color) in &mut texts {
        let (wanted, tint) = match *part {
            Part::MarkerLetter(i) => list.get(i as usize).map_or((String::new(), Color::WHITE), |m| (m.letter.clone(), m.color)),
            Part::MarkerText(i) => list.get(i as usize).map_or((String::new(), ui::TEXT), |m| (m.line.clone(), ui::TEXT)),
            _ => continue,
        };
        set(&mut text.0, wanted);
        set(&mut color.0, tint);
    }
}

/// The points (and the bomb) on the radar, pinned to its rim when out of range.
#[allow(clippy::too_many_arguments)]
fn minimap_icons(
    mut commands: Commands,
    time: Res<Time>,
    state: Res<MatchState>,
    pivots: Query<(Entity, Option<&Children>), With<crate::tactical::MinimapPivot>>,
    local: Query<(&Player, &Transform), With<LocalPlayer>>,
    mut icons: Query<(&Part, &mut Node, &mut BorderColor, &mut Visibility), Without<Text>>,
    mut letters: Query<(&Part, &mut Text, &mut TextColor)>,
) {
    let Ok((pivot, children)) = pivots.single() else { return };
    let Ok((me, transform)) = local.single() else { return };
    // The baked map image goes in first (see `tactical::minimap`); icons ride on top of it.
    if children.is_none_or(|c| c.is_empty()) {
        return;
    }
    if !icons.iter().any(|(part, ..)| matches!(part, Part::Mini(_))) {
        for i in 0..MARKERS {
            commands
                .spawn((
                    Part::Mini(i),
                    Node { position_type: PositionType::Absolute, min_width: Val::Px(13.0), height: Val::Px(13.0), border: UiRect::all(Val::Px(1.0)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
                    BackgroundColor(Color::srgba(0.02, 0.04, 0.06, 0.8)),
                    BorderColor(NEUTRAL_COLOR),
                    Visibility::Hidden,
                    ChildOf(pivot),
                ))
                .with_children(|icon| {
                    icon.spawn((Part::MiniLetter(i), Text::new(""), TextFont { font_size: 9.0, ..default() }, TextColor(Color::WHITE)));
                });
        }
        return;
    }
    let list = if state.obj.kind != kind::NONE { marks(&state, me, transform.translation, time.elapsed_secs()) } else { Vec::new() };
    let scale = (crate::hud::RADAR / 2.0 - 4.0) / crate::hud::RADAR_RANGE;
    for (part, mut node, mut border, mut visibility) in &mut icons {
        let Part::Mini(i) = *part else { continue };
        let Some(mark) = list.get(i as usize) else {
            set(&mut *visibility, Visibility::Hidden);
            continue;
        };
        set(&mut *visibility, Visibility::Inherited);
        let offset = (mark.at.xz() - transform.translation.xz()).clamp_length_max(crate::hud::RADAR_RANGE * 0.9) * scale;
        let half = 7.0;
        set(&mut node.left, Val::Px((offset.x - half).round()));
        set(&mut node.top, Val::Px((offset.y - 7.0).round()));
        set(&mut border.0, mark.color);
    }
    for (part, mut text, mut color) in &mut letters {
        let Part::MiniLetter(i) = *part else { continue };
        let (wanted, tint) = list.get(i as usize).map_or((String::new(), Color::WHITE), |m| (if m.letter.len() > 1 { "!".into() } else { m.letter.clone() }, m.color));
        set(&mut text.0, wanted);
        set(&mut color.0, tint);
    }
}

/// Rings and flag poles on the points, and the bomb lying (or ticking) where it is.
#[allow(clippy::too_many_arguments)]
fn props(
    mut commands: Commands,
    time: Res<Time>,
    state: Res<MatchState>,
    mut props: ResMut<Props>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    spawned: Query<Entity, With<PointProp>>,
    mut bomb_prop: Query<(&mut Transform, &mut Visibility), With<BombProp>>,
) {
    let obj = &state.obj;
    let wanted: Vec<Vec3> = obj.points.iter().map(|p| p.at).collect();
    let blink = (time.elapsed_secs() * if obj.fuse < 10.0 { 6.0 } else { 2.0 }).fract() < 0.5;
    if props.spawned != wanted {
        for entity in &spawned {
            commands.entity(entity).despawn();
        }
        props.paints.clear();
        let flat = |radius: f32| Mesh::from(Annulus::new(radius - 0.25, radius));
        let ring = meshes.add(flat(if obj.kind == kind::DOMINATION { POINT_RADIUS } else { SITE_RADIUS }));
        let pole = meshes.add(Mesh::from(Cylinder::new(0.05, 3.2)));
        let cloth = meshes.add(Mesh::from(Cuboid::new(0.9, 0.55, 0.04)));
        let grey = materials.add(StandardMaterial { base_color: Color::srgb(0.55, 0.57, 0.6), perceptual_roughness: 0.7, ..default() });
        for at in &wanted {
            let paint = materials.add(StandardMaterial { base_color: NEUTRAL_COLOR.with_alpha(0.7), unlit: true, alpha_mode: AlphaMode::Blend, double_sided: true, cull_mode: None, ..default() });
            props.paints.push(paint.clone());
            let lift = Vec3::Y * 0.06;
            commands.spawn((MatchEntity, PointProp, Mesh3d(ring.clone()), MeshMaterial3d(paint.clone()), Transform::from_translation(*at + lift).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)), bevy::pbr::NotShadowCaster));
            commands.spawn((MatchEntity, PointProp, Mesh3d(pole.clone()), MeshMaterial3d(grey.clone()), Transform::from_translation(*at + Vec3::Y * 1.6)));
            commands.spawn((MatchEntity, PointProp, Mesh3d(cloth.clone()), MeshMaterial3d(paint), Transform::from_translation(*at + Vec3::new(0.47, 2.85, 0.0)), bevy::pbr::NotShadowCaster));
        }
        props.spawned = wanted;
    }
    for (i, paint) in props.paints.iter().enumerate() {
        let Some(point) = obj.points.get(i) else { continue };
        let color = match obj.kind {
            kind::DOMINATION if point.contested && blink => ui::WARN,
            kind::DOMINATION => team_color(point.owner),
            _ if obj.bomb == bomb::PLANTED && obj.site as usize == i && blink => ui::WARN,
            _ => BOMB_COLOR,
        };
        if let Some(material) = materials.get_mut(paint) {
            let wanted = color.with_alpha(0.75);
            if material.base_color != wanted {
                material.base_color = wanted;
            }
        }
    }
    // The bomb: a satchel on the floor, blinking once planted.
    let Ok((mut transform, mut visibility)) = bomb_prop.single_mut() else {
        if obj.kind == kind::BOMB {
            let paint = materials.add(StandardMaterial { base_color: Color::srgb(0.25, 0.22, 0.2), emissive: LinearRgba::rgb(0.0, 0.0, 0.0), perceptual_roughness: 0.8, ..default() });
            props.bomb = Some(paint.clone());
            commands.spawn((MatchEntity, BombProp, Mesh3d(meshes.add(Mesh::from(Cuboid::new(0.5, 0.26, 0.34)))), MeshMaterial3d(paint), Transform::default(), Visibility::Hidden));
        }
        return;
    };
    let lying = obj.kind == kind::BOMB && matches!(obj.bomb, bomb::HOME | bomb::DROPPED | bomb::PLANTED) && matches!(state.phase, Phase::Live);
    set(&mut *visibility, if lying { Visibility::Inherited } else { Visibility::Hidden });
    if lying {
        transform.translation = obj.bomb_at + Vec3::Y * 0.13;
        if let Some(material) = props.bomb.as_ref().and_then(|h| materials.get_mut(h)) {
            let glow = if obj.bomb == bomb::PLANTED && blink { LinearRgba::rgb(4.0, 0.3, 0.1) } else { LinearRgba::rgb(0.6, 0.25, 0.0) };
            if material.emissive != glow {
                material.emissive = glow;
            }
        }
    }
}
