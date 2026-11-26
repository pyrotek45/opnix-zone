//! The heads-up display, drawn as shapes rather than sentences.
//!
//! Health and beacon integrity are segmented bars, ammunition is a row of
//! rounds, the day/night cycle is a sun sliding along a track, hostiles and
//! team scores are bars, and a radar shows what is around you. Numbers only
//! appear where a count genuinely matters. Every element is a plain coloured
//! rectangle, updated only when its value changes.

use crate::{ClientSet, input::Controls, stations::{Nearby, Placement}, ui::{self, Screen}, view::ViewState};
use bevy::{
    asset::RenderAssetUsages,
    ecs::hierarchy::ChildSpawnerCommands,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use oz_core::arsenal::resolve;
use oz_sim::prelude::*;

const HEALTH_SEGMENTS: u8 = 10;
const BEACON_SEGMENTS: u8 = 20;
const AMMO_PIPS: u8 = 30;
const SLOTS: u8 = 8;
const GEAR_PIPS: u8 = 4;
const BLIPS: u8 = 40;
const RADAR: f32 = 124.0;
const RADAR_RANGE: f32 = 48.0;
const TRACK: f32 = 260.0;
const SCORE_TRACK: f32 = 190.0;

const OFF: Color = Color::srgba(0.10, 0.14, 0.17, 0.75);
const FRAME: Color = Color::srgba(0.02, 0.04, 0.06, 0.72);
const AMBER: Color = Color::srgb(1.0, 0.80, 0.40);
const RED: Color = Color::srgb(1.0, 0.33, 0.37);
const TEAL: Color = Color::srgb(0.25, 0.80, 0.82);
const CORAL: Color = Color::srgb(0.95, 0.52, 0.42);
const ORE: [Color; 3] = [Color::srgb(0.62, 0.65, 0.72), Color::srgb(0.90, 0.55, 0.30), Color::srgb(0.62, 0.97, 0.89)];

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

/// One lamp in a bar, a pip, a radar blip or a crosshair tick.
#[derive(Component, Clone, Copy)]
enum Seg {
    Health(u8),
    Beacon(u8),
    Ammo(u8),
    Slot(u8),
    Gear(u8),
    Blip(u8),
    Tick(u8),
    /// The sun / moon marker riding the day track.
    Orb,
}

/// A bar whose width is a fraction of its track.
#[derive(Component, Clone, Copy)]
enum Fill {
    Phase,
    ScoreA,
    ScoreB,
}

#[derive(Component, Clone, Copy)]
enum Num {
    Health,
    Ammo,
    Reserve,
    Weapon,
    Credits,
    Ore(u8),
    Day,
    ScoreA,
    ScoreB,
    Notice,
    Prompt,
    Hint,
}

/// Groups that only exist in some modes.
#[derive(Component, Clone, Copy, PartialEq)]
enum Zone {
    Coop,
    Beacon,
    Versus,
}

#[derive(Component)]
struct HudRoot;

#[derive(Component)]
struct NetStats;

/// Full-screen scope mask, shown while aiming through magnified optics.
#[derive(Component)]
struct ScopeOverlay;

#[derive(Component)]
struct ScopeGlass;

/// One limb of the little stance figure: 0 head, 1 body, 2 legs.
#[derive(Component, Clone, Copy)]
struct StanceLimb(u8);

#[derive(Resource)]
struct ScopeImages([Handle<Image>; 2]);

/// A round scope mask: black outside the glass, a reticle inside. `kind` 1 = ACOG chevron, 2 = sniper crosshair.
fn scope_image(kind: u8) -> Image {
    let size = 256usize;
    let c = size as f32 / 2.0;
    let mut pixels = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let d = (dx * dx + dy * dy).sqrt() / c;
            let mut rgba = [0u8, 0, 0, 0];
            if d > 0.97 {
                rgba = [0, 0, 0, 255];
            } else if d > 0.93 {
                rgba = [18, 22, 26, 255];
            } else if d > 0.86 {
                // Vignette inside the rim.
                rgba = [0, 0, 0, ((d - 0.86) / 0.07 * 170.0) as u8];
            }
            let line = |v: f32, w: f32| v.abs() < w;
            if kind == 2 {
                // Heavy posts from the edge, fine cross in the middle, mil dots.
                let post = (line(dx, 3.0) && dy.abs() > c * 0.45) || (line(dy, 3.0) && dx.abs() > c * 0.45);
                let fine = (line(dx, 0.7) || line(dy, 0.7)) && d < 0.93;
                let dot = (line(dx, 1.5) || line(dy, 1.5)) && ((dx.abs().max(dy.abs()) / (c * 0.09)).fract() < 0.12) && dx.abs().max(dy.abs()) > 4.0 && d < 0.45;
                if (post && d < 0.93) || fine || dot {
                    rgba = [8, 8, 10, 255];
                }
            } else if kind == 1 {
                // An amber chevron with a fine stadia line below it.
                let up = dy - 2.0;
                let chevron = up >= 0.0 && up < c * 0.12 && (dx.abs() - up * 0.9).abs() < 1.6;
                let stadia = line(dx, 0.7) && dy > c * 0.14 && dy < c * 0.6;
                if chevron {
                    rgba = [255, 190, 70, 255];
                } else if stadia || (line(dy, 0.7) && dx.abs() > c * 0.55 && d < 0.93) {
                    rgba = [10, 10, 10, 230];
                }
            }
            pixels[(y * size + x) * 4..(y * size + x) * 4 + 4].copy_from_slice(&rgba);
        }
    }
    let mut image = Image::new(Extent3d { width: size as u32, height: size as u32, depth_or_array_layers: 1 }, TextureDimension::D2, pixels, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

#[derive(Component)]
struct HurtOverlay;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Notices>()
            .add_systems(OnEnter(MatchPhase::Active), spawn_hud)
            .add_systems(Update, (update_text, update_shapes, scope_and_stance, net_stats).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

fn block(width: f32, height: f32, color: Color) -> impl Bundle {
    (Node { width: Val::Px(width), height: Val::Px(height), ..default() }, BackgroundColor(color))
}

fn at(node: Node) -> Node {
    Node { position_type: PositionType::Absolute, ..node }
}

fn number(parent: &mut ChildSpawnerCommands, kind: Num, size: f32, color: Color) {
    parent.spawn((kind, Text::new(""), TextFont { font_size: size, ..default() }, TextColor(color)));
}

fn row(gap: f32) -> Node {
    Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(gap), align_items: AlignItems::Center, ..default() }
}

fn column(gap: f32, align: AlignItems) -> Node {
    Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(gap), align_items: align, ..default() }
}

fn framed(node: Node) -> impl Bundle {
    (Node { padding: UiRect::all(Val::Px(8.0)), border: UiRect::all(Val::Px(2.0)), ..node }, BackgroundColor(FRAME), BorderColor(Color::srgba(0.30, 0.40, 0.42, 0.8)))
}

fn spawn_hud(mut commands: Commands, mut notices: ResMut<Notices>, mut images: ResMut<Assets<Image>>) {
    notices.0.clear();
    let scopes = [images.add(scope_image(1)), images.add(scope_image(2))];
    commands
        .spawn((
            MatchEntity,
            ScopeOverlay,
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
            GlobalZIndex(-45),
            Visibility::Hidden,
        ))
        .with_children(|overlay| {
            overlay.spawn((Node { flex_grow: 1.0, height: Val::Percent(100.0), ..default() }, BackgroundColor(Color::BLACK)));
            overlay.spawn((ScopeGlass, ImageNode::new(scopes[1].clone()), Node { height: Val::Percent(100.0), aspect_ratio: Some(1.0), ..default() }));
            overlay.spawn((Node { flex_grow: 1.0, height: Val::Percent(100.0), ..default() }, BackgroundColor(Color::BLACK)));
        });
    commands.insert_resource(ScopeImages(scopes));
    commands.spawn((
        MatchEntity,
        NetStats,
        Text::new(""),
        TextFont { font_size: 12.0, ..default() },
        TextColor(ui::DIM),
        Node { position_type: PositionType::Absolute, right: Val::Px(150.0), top: Val::Px(20.0), ..default() },
        GlobalZIndex(-30),
    ));
    let full = || at(Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() });
    commands.spawn((MatchEntity, HurtOverlay, full(), BackgroundColor(Color::NONE), GlobalZIndex(40)));
    commands.spawn((MatchEntity, HudRoot, full(), GlobalZIndex(-40))).with_children(|hud| {
        // Bottom left: health as a segmented bar behind a medical cross, plus gear pips.
        hud.spawn(framed(at(Node { left: Val::Px(20.0), bottom: Val::Px(20.0), ..column(6.0, AlignItems::Start) }))).with_children(|panel| {
            panel.spawn(row(3.0)).with_children(|bar| {
                bar.spawn(Node { width: Val::Px(16.0), height: Val::Px(16.0), margin: UiRect::right(Val::Px(5.0)), ..default() }).with_children(|cross| {
                    cross.spawn((at(Node { left: Val::Px(5.0), width: Val::Px(6.0), height: Val::Px(16.0), ..default() }), BackgroundColor(RED)));
                    cross.spawn((at(Node { top: Val::Px(5.0), width: Val::Px(16.0), height: Val::Px(6.0), ..default() }), BackgroundColor(RED)));
                });
                for i in 0..HEALTH_SEGMENTS {
                    bar.spawn((Seg::Health(i), block(15.0, 14.0, OFF)));
                }
                number(bar, Num::Health, 15.0, ui::TEXT);
            });
            panel.spawn(row(10.0)).with_children(|line| {
                // A tiny figure that stands, crouches or lies down with you.
                line.spawn(Node { width: Val::Px(26.0), height: Val::Px(24.0), ..default() }).with_children(|figure| {
                    for limb in 0..3 {
                        figure.spawn((StanceLimb(limb), at(Node::default()), BackgroundColor(ui::TEXT)));
                    }
                });
                line.spawn(row(4.0)).with_children(|gear| {
                    for i in 0..GEAR_PIPS {
                        gear.spawn((Seg::Gear(i), block(10.0, 10.0, OFF)));
                    }
                });
            });
        });

        // Bottom right: weapon slots, the round counter, and one pip per round.
        hud.spawn(framed(at(Node { right: Val::Px(20.0), bottom: Val::Px(20.0), ..column(5.0, AlignItems::End) }))).with_children(|panel| {
            number(panel, Num::Weapon, 13.0, ui::DIM);
            panel.spawn(Node { align_items: AlignItems::End, column_gap: Val::Px(8.0), ..default() }).with_children(|count| {
                number(count, Num::Ammo, 34.0, ui::TEXT);
                number(count, Num::Reserve, 16.0, ui::DIM);
            });
            panel.spawn(Node { width: Val::Px(AMMO_PIPS as f32 * 6.0), column_gap: Val::Px(2.0), ..default() }).with_children(|pips| {
                for i in 0..AMMO_PIPS {
                    pips.spawn((Seg::Ammo(i), block(4.0, 11.0, OFF)));
                }
            });
            panel.spawn(row(3.0)).with_children(|slots| {
                for i in 0..SLOTS {
                    slots.spawn((Seg::Slot(i), Node { width: Val::Px(20.0), height: Val::Px(7.0), ..default() }, BackgroundColor(OFF)));
                }
            });
        });

        // Top left: what the team owns, each with its own colour chip.
        hud.spawn((Zone::Coop, framed(at(Node { left: Val::Px(20.0), top: Val::Px(18.0), ..column(4.0, AlignItems::Start) })))).with_children(|panel| {
            let mut line = |kind: Num, chip: Color, round: bool| {
                panel.spawn(row(8.0)).with_children(|line| {
                    line.spawn((block(12.0, 12.0, chip), BorderRadius::all(Val::Px(if round { 6.0 } else { 0.0 }))));
                    number(line, kind, 16.0, ui::TEXT);
                });
            };
            line(Num::Credits, AMBER, true);
            for ore in 0..3 {
                line(Num::Ore(ore), ORE[ore as usize], false);
            }
        });

        // Top centre: the day track (a sun that slides to dusk, then the hostile bar), the beacon, or the scores.
        hud.spawn(at(Node { width: Val::Percent(100.0), top: Val::Px(16.0), justify_content: JustifyContent::Center, ..default() })).with_children(|top| {
            top.spawn(column(6.0, AlignItems::Center)).with_children(|stack| {
                stack.spawn((Zone::Coop, row(8.0))).with_children(|phase| {
                    number(phase, Num::Day, 18.0, ui::TEXT);
                    phase.spawn((Node { width: Val::Px(TRACK), height: Val::Px(12.0), ..default() }, BackgroundColor(OFF))).with_children(|track| {
                        track.spawn((Fill::Phase, at(Node { height: Val::Px(12.0), ..default() }), BackgroundColor(AMBER)));
                        track.spawn((Seg::Orb, at(Node { top: Val::Px(-4.0), width: Val::Px(20.0), height: Val::Px(20.0), ..default() }), BackgroundColor(AMBER), BorderRadius::all(Val::Px(10.0))));
                    });
                });
                stack.spawn((Zone::Beacon, row(2.0))).with_children(|beacon| {
                    beacon.spawn((Node { width: Val::Px(12.0), height: Val::Px(12.0), margin: UiRect::right(Val::Px(6.0)), ..default() }, BackgroundColor(ui::ACCENT)));
                    for i in 0..BEACON_SEGMENTS {
                        beacon.spawn((Seg::Beacon(i), block(11.0, 9.0, OFF)));
                    }
                });
                stack.spawn((Zone::Versus, row(8.0))).with_children(|scores| {
                    number(scores, Num::ScoreA, 24.0, TEAL);
                    for (fill, color, justify) in [(Fill::ScoreA, TEAL, JustifyContent::End), (Fill::ScoreB, CORAL, JustifyContent::Start)] {
                        scores.spawn((Node { width: Val::Px(SCORE_TRACK), height: Val::Px(14.0), justify_content: justify, ..default() }, BackgroundColor(OFF))).with_children(|track| {
                            track.spawn((fill, Node { height: Val::Px(14.0), ..default() }, BackgroundColor(color)));
                        });
                    }
                    number(scores, Num::ScoreB, 24.0, CORAL);
                });
            });
        });

        // Top right: radar. Up is the way you are facing.
        hud.spawn((at(Node { right: Val::Px(20.0), top: Val::Px(18.0), width: Val::Px(RADAR), height: Val::Px(RADAR), border: UiRect::all(Val::Px(2.0)), ..default() }), BackgroundColor(FRAME), BorderColor(Color::srgba(0.30, 0.40, 0.42, 0.8))))
            .with_children(|radar| {
                let middle = RADAR / 2.0 - 2.0;
                radar.spawn((at(Node { left: Val::Px(middle - 1.0), top: Val::Px(middle - 3.0), width: Val::Px(2.0), height: Val::Px(8.0), ..default() }), BackgroundColor(ui::TEXT)));
                radar.spawn((at(Node { left: Val::Px(middle - 3.0), top: Val::Px(middle), width: Val::Px(6.0), height: Val::Px(3.0), ..default() }), BackgroundColor(ui::TEXT)));
                for i in 0..BLIPS {
                    radar.spawn((Seg::Blip(i), at(Node { width: Val::Px(5.0), height: Val::Px(5.0), ..default() }), BackgroundColor(Color::NONE)));
                }
            });

        // Crosshair: four ticks that spread with movement and recoil.
        hud.spawn(at(Node { left: Val::Percent(50.0), top: Val::Percent(50.0), ..default() })).with_children(|cross| {
            for i in 0..4 {
                cross.spawn((Seg::Tick(i), at(Node::default()), BackgroundColor(ui::TEXT)));
            }
        });

        let centered = |top: f32| at(Node { width: Val::Percent(100.0), top: Val::Percent(top), justify_content: JustifyContent::Center, ..default() });
        for (kind, top, size, color) in [(Num::Notice, 21.0, 21.0, ui::TEXT), (Num::Prompt, 62.0, 19.0, ui::ACCENT), (Num::Hint, 85.0, 13.0, ui::DIM)] {
            hud.spawn(centered(top)).with_children(|c| {
                c.spawn((kind, Text::new(""), TextFont { font_size: size, ..default() }, TextColor(color), TextLayout::new_with_justify(JustifyText::Center)));
            });
        }
    });
}

fn set<T: PartialEq>(slot: &mut T, value: T) {
    if *slot != value {
        *slot = value;
    }
}

#[allow(clippy::too_many_arguments)]
fn update_text(
    time: Res<Time<Real>>,
    state: Res<MatchState>,
    content: Res<Content>,
    rules: Res<Rules>,
    controls: Res<Controls>,
    nearby: Res<Nearby>,
    placing: Res<Placement>,
    screen: Res<Screen>,
    mut notices: ResMut<Notices>,
    spectate: Res<crate::spectate::Spectate>,
    (death, prediction, role, settings): (Res<crate::fx::DeathInfo>, Res<crate::predict::Prediction>, Res<NetRole>, Res<crate::settings::Settings>),
    players: Query<(&Health, &Arsenal, &Player), With<LocalPlayer>>,
    mut texts: Query<(&Num, &mut Text)>,
    mut root: Query<&mut Visibility, With<HudRoot>>,
    mut shown_for: Local<f32>,
) {
    let dt = time.delta_secs();
    notices.0.retain_mut(|(_, ttl)| {
        *ttl -= dt;
        *ttl > 0.0
    });
    if let Ok(mut visibility) = root.single_mut() {
        set(&mut *visibility, if matches!(*screen, Screen::None | Screen::Chat) { Visibility::Inherited } else { Visibility::Hidden });
    }
    let Ok((health, arsenal, player)) = players.single() else { return };
    *shown_for = if state.is_added() { 0.0 } else { *shown_for + dt };
    let index = arsenal.current as usize;
    let weapon = content.weapons.get(index).zip(arsenal.weapons.get(index));
    let pending = if *role == NetRole::Client { prediction.pending } else { 0 };

    use crate::bindings::Action;
    let key = |action| settings.bindings.label(action);
    let (use_key, fire_key, cancel_key, rotate_key): (String, String, String, String) = if controls.gamepad { ("X".into(), "RT".into(), "B".into(), "X".into()) } else { (key(Action::Interact), key(Action::Fire), "ESC".into(), key(Action::Reload)) };
    let killer = death.killer.and_then(|id| state.roster.iter().find(|r| r.id == id)).map(|r| r.name.clone());
    let prompt = if !health.alive() {
        match killer {
            Some(name) => format!("KILLED BY {name}  -  BACK IN {:.0}", player.respawn.ceil()),
            None => format!("DOWN  -  BACK IN {:.0}", player.respawn.ceil()),
        }
    } else if spectate.enabled {
        String::new()
    } else if player.protect > 0.0 {
        "SPAWN PROTECTED".to_string()
    } else if let Some(kind) = placing.kind {
        let name = content.structures.get(kind as usize).map_or("STRUCTURE", |s| s.name);
        format!("{name}\n[{fire_key}] BUILD   [{rotate_key}] ROTATE   [{cancel_key}] DONE")
    } else if let Some((_, kind)) = nearby.0 {
        format!("[{use_key}]  {}", content.stations.get(kind as usize).map_or("STATION", |s| s.name))
    } else if weapon.is_some_and(|(def, owned)| def.mag > 0 && owned.ammo == 0 && arsenal.reload <= 0.0) {
        format!("[{}]  RELOAD", if controls.gamepad { "X".into() } else { key(Action::Reload) })
    } else {
        String::new()
    };
    // Controls are shown while a match is young, then get out of the way.
    let help = if rules.pvp {
        format!(
            "{} FIRE   {} AIM   {} RELOAD   {}/{} LEAN   {} GEAR   {} CLASS   {} SPRINT   ESC PAUSE",
            key(Action::Fire), key(Action::Aim), key(Action::Reload), key(Action::LeanLeft), key(Action::LeanRight), key(Action::Gear), key(Action::Loadout), key(Action::Sprint)
        )
    } else {
        format!("{} FIRE   {} AIM   {} RELOAD   {} USE   {}/{} LEAN   1-8 WEAPON   {} SPRINT   ESC PAUSE", key(Action::Fire), key(Action::Aim), key(Action::Reload), key(Action::Interact), key(Action::LeanLeft), key(Action::LeanRight), key(Action::Sprint))
    };
    let watching = format!("SPECTATING  {}      [V] BACK TO YOUR PLAYER", spectate.name);
    let hint = if spectate.enabled {
        watching.as_str()
    } else if *shown_for > 30.0 {
        ""
    } else if controls.gamepad {
        "STICKS MOVE / LOOK    RT FIRE    LT AIM    X RELOAD / USE    A JUMP    RB GEAR    D-PAD LEAN    Y / LB WEAPON    START PAUSE"
    } else {
        help.as_str()
    };

    for (kind, mut text) in &mut texts {
        let wanted = match kind {
            Num::Health => format!("{:.0}", health.hp.max(0.0)),
            Num::Ammo => weapon.map_or(String::new(), |(def, owned)| if def.mag == 0 { String::new() } else { owned.ammo.saturating_sub(pending).to_string() }),
            Num::Reserve => weapon.map_or(String::new(), |(def, owned)| if def.mag == 0 { String::new() } else if def.infinite_reserve { "INF".into() } else { owned.reserve.to_string() }),
            Num::Weapon => weapon.map_or(String::new(), |(def, _)| def.name.to_string()),
            Num::Credits => state.stock.credits.to_string(),
            Num::Ore(ore) => state.stock.ore[*ore as usize].to_string(),
            Num::Day => format!("{:02}", state.day),
            Num::ScoreA => state.scores[0].to_string(),
            Num::ScoreB => state.scores[1].to_string(),
            Num::Notice => notices.0.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n"),
            Num::Prompt => prompt.clone(),
            Num::Hint => hint.to_string(),
        };
        set(&mut text.0, wanted);
    }
}

#[allow(clippy::too_many_arguments)]
fn update_shapes(
    state: Res<MatchState>,
    content: Res<Content>,
    rules: Res<Rules>,
    view: Res<ViewState>,
    spectate: Res<crate::spectate::Spectate>,
    (prediction, role): (Res<crate::predict::Prediction>, Res<NetRole>),
    players: Query<(&Health, &Arsenal, &Player, &Transform, &PlayerInput, &Mover), With<LocalPlayer>>,
    enemies: Query<&Transform, With<Enemy>>,
    others: Query<(&Transform, &Player, &Health), Without<LocalPlayer>>,
    stations: Query<&Transform, With<Station>>,
    mut segments: Query<(&Seg, &mut BackgroundColor, &mut Node), (Without<Fill>, Without<Zone>)>,
    mut fills: Query<(&Fill, &mut Node, &mut BackgroundColor), (Without<Seg>, Without<Zone>)>,
    mut zones: Query<(&Zone, &mut Node), (Without<Seg>, Without<Fill>)>,
    mut hurt: Query<&mut BackgroundColor, (With<HurtOverlay>, Without<Seg>, Without<Fill>)>,
    mut wave_size: Local<u32>,
) {
    let Ok((health, arsenal, me, transform, input, mover)) = players.single() else { return };
    if let Ok(mut overlay) = hurt.single_mut() {
        let low = if health.alive() { (0.35 - health.fraction()).max(0.0) * 0.6 } else { 0.45 };
        let color = if view.blind > 0.0 {
            // A flashbang whites the screen out, then fades.
            Color::srgba(1.0, 1.0, 0.96, view.blind.min(1.0).powf(0.7))
        } else {
            Color::srgba(0.8, 0.05, 0.05, (view.hurt * 0.16 + low * 0.6).min(0.32))
        };
        set(&mut overlay.0, color);
    }
    for (zone, mut node) in &mut zones {
        let on = match zone {
            Zone::Coop => !rules.pvp,
            Zone::Beacon => state.objective_max > 0.0,
            Zone::Versus => rules.pvp,
        };
        set(&mut node.display, if on { Display::Flex } else { Display::None });
    }

    // Day: the bar drains toward nightfall. Night: it shows how much of the wave is left.
    let night = state.phase == Phase::Night;
    *wave_size = if night { (*wave_size).max(state.enemies_left) } else { 0 };
    let phase_fraction = if night {
        state.enemies_left as f32 / (*wave_size).max(1) as f32
    } else if rules.day_seconds > 0.0 {
        (state.timer / rules.day_seconds).clamp(0.0, 1.0)
    } else {
        (state.timer / 12.0).clamp(0.0, 1.0)
    };
    let limit = rules.score_limit.max(1) as f32;
    for (fill, mut node, mut color) in &mut fills {
        let (width, tint) = match fill {
            Fill::Phase => (TRACK * phase_fraction, if night { RED } else { AMBER }),
            Fill::ScoreA => (SCORE_TRACK * (state.scores[0] as f32 / limit).min(1.0), TEAL),
            Fill::ScoreB => (SCORE_TRACK * (state.scores[1] as f32 / limit).min(1.0), CORAL),
        };
        set(&mut node.width, Val::Px(width.round()));
        set(&mut color.0, tint);
    }

    // Radar contacts, rotated so "up" is forward.
    let forward = Vec2::new(-input.yaw.sin(), -input.yaw.cos());
    let right = Vec2::new(input.yaw.cos(), -input.yaw.sin());
    let here = transform.translation.xz();
    let mut contacts: Vec<(Vec2, Color)> = Vec::with_capacity(BLIPS as usize);
    let mut contact = |at: Vec3, color: Color, range: f32| {
        let offset = at.xz() - here;
        if offset.length() < range && contacts.len() < BLIPS as usize {
            contacts.push((Vec2::new(offset.dot(right), offset.dot(forward)) / RADAR_RANGE, color));
        }
    };
    for (other, player, other_health) in &others {
        if other_health.alive() {
            let friend = player.team == me.team;
            // Opponents only show when they are close enough to hear.
            contact(other.translation, if player.team == 0 { TEAL } else { CORAL }, if friend { RADAR_RANGE } else { 16.0 });
        }
    }
    for station in &stations {
        contact(station.translation, AMBER, RADAR_RANGE);
    }
    for enemy in &enemies {
        contact(enemy.translation, RED, RADAR_RANGE);
    }

    let index = arsenal.current as usize;
    let weapon = content.weapons.get(index).zip(arsenal.weapons.get(index));
    let (mag, ammo, reload) = weapon.map_or((0, 0, 0.0), |(def, owned)| {
        let stats = resolve(def, owned, &content.attachments);
        let pending = if *role == NetRole::Client { prediction.pending } else { 0 };
        (if def.mag == 0 { 0 } else { stats.mag }, owned.ammo.saturating_sub(pending), if arsenal.reload > 0.0 { 1.0 - arsenal.reload / stats.reload.max(0.01) } else { 0.0 })
    });
    let pips = mag.min(AMMO_PIPS as u16) as f32;
    let lit_pips = if reload > 0.0 { reload * pips } else if mag > 0 { (ammo as f32 / mag as f32 * pips).ceil() } else { 0.0 };
    let health_lit = (health.fraction() * HEALTH_SEGMENTS as f32).ceil() as u8;
    let health_color = if health.fraction() > 0.5 { ui::GOOD } else if health.fraction() > 0.25 { AMBER } else { RED };
    let beacon_fraction = if state.objective_max > 0.0 { state.objective_hp / state.objective_max } else { 0.0 };
    let beacon_lit = (beacon_fraction * BEACON_SEGMENTS as f32).ceil() as u8;
    let owned: Vec<usize> = (0..arsenal.weapons.len()).filter(|i| arsenal.owns(*i)).collect();
    let gap = 5.0 + (mover.speed * 1.1).min(9.0) + view.recoil * 9.0 + if view.hit > 0.0 { 3.0 } else { 0.0 };
    let cross_color = if view.hit > 0.0 { if view.kill { RED } else { AMBER } } else { ui::TEXT };
    let show_cross = health.alive() && view.ads < 0.85 && !spectate.enabled;

    for (segment, mut color, mut node) in &mut segments {
        let tint = match *segment {
            Seg::Health(i) => if i < health_lit { health_color } else { OFF },
            Seg::Beacon(i) => if i < beacon_lit { if beacon_fraction < 0.25 { RED } else { ui::ACCENT } } else { OFF },
            Seg::Ammo(i) => {
                set(&mut node.display, if (i as f32) < pips { Display::Flex } else { Display::None });
                if (i as f32) < lit_pips { if reload > 0.0 { ui::ACCENT } else if ammo as f32 <= mag as f32 * 0.25 { RED } else { AMBER } } else { OFF }
            }
            Seg::Slot(i) => {
                set(&mut node.display, if (i as usize) < owned.len() { Display::Flex } else { Display::None });
                if owned.get(i as usize) == Some(&index) { AMBER } else { OFF }
            }
            Seg::Gear(i) => {
                set(&mut node.display, if i < me.gear_left { Display::Flex } else { Display::None });
                CORAL
            }
            Seg::Blip(i) => match contacts.get(i as usize) {
                Some((offset, tint)) => {
                    let half = RADAR / 2.0 - 4.0;
                    set(&mut node.left, Val::Px((half + offset.x * half - 1.0).round()));
                    set(&mut node.top, Val::Px((half - offset.y * half - 1.0).round()));
                    *tint
                }
                None => Color::NONE,
            },
            Seg::Tick(i) => {
                // Left, right, up, down.
                let (x, y, w, h) = match i {
                    0 => (-gap - 8.0, -1.0, 8.0, 2.0),
                    1 => (gap, -1.0, 8.0, 2.0),
                    2 => (-1.0, -gap - 8.0, 2.0, 8.0),
                    _ => (-1.0, gap, 2.0, 8.0),
                };
                set(&mut node.left, Val::Px(x.round()));
                set(&mut node.top, Val::Px(y.round()));
                set(&mut node.width, Val::Px(w));
                set(&mut node.height, Val::Px(h));
                if show_cross { cross_color } else { Color::NONE }
            }
            Seg::Orb => {
                // The sun slides from dawn to dusk; at night a pale moon sits at the end.
                let travel = if night { 1.0 } else { 1.0 - phase_fraction };
                set(&mut node.left, Val::Px((travel * (TRACK - 20.0)).round()));
                if night { Color::srgb(0.75, 0.85, 1.0) } else { Color::srgb(1.0, 0.93, 0.6) }
            }
        };
        set(&mut color.0, tint);
    }
}

/// Scope mask while aiming through magnified glass, and the stance figure.
fn scope_and_stance(
    view: Res<ViewState>,
    screen: Res<Screen>,
    images: Option<Res<ScopeImages>>,
    players: Query<&PlayerInput, With<LocalPlayer>>,
    mut overlay: Query<&mut Visibility, With<ScopeOverlay>>,
    mut glass: Query<&mut ImageNode, With<ScopeGlass>>,
    mut limbs: Query<(&StanceLimb, &mut Node)>,
) {
    if let Ok(mut visibility) = overlay.single_mut() {
        let wanted = if view.scoped > 0.5 && *screen == Screen::None { Visibility::Inherited } else { Visibility::Hidden };
        set(&mut *visibility, wanted);
    }
    if let (Ok(mut glass), Some(images)) = (glass.single_mut(), images) {
        let handle = &images.0[if view.scope_kind >= 2 { 1 } else { 0 }];
        if glass.image != *handle {
            glass.image = handle.clone();
        }
    }
    let Ok(input) = players.single() else { return };
    for (limb, mut node) in &mut limbs {
        // (left, top, width, height) for head, body, legs in each stance.
        let (x, y, w, h) = match (input.stance, limb.0) {
            (stance::PRONE, 0) => (20.0, 14.0, 6.0, 6.0),
            (stance::PRONE, 1) => (8.0, 16.0, 12.0, 5.0),
            (stance::PRONE, _) => (0.0, 17.0, 8.0, 4.0),
            (stance::CROUCH, 0) => (12.0, 4.0, 6.0, 6.0),
            (stance::CROUCH, 1) => (11.0, 10.0, 6.0, 7.0),
            (stance::CROUCH, _) => (7.0, 17.0, 10.0, 5.0),
            (_, 0) => (10.0, 0.0, 6.0, 6.0),
            (_, 1) => (10.0, 7.0, 6.0, 9.0),
            _ => (10.0, 16.0, 6.0, 8.0),
        };
        set(&mut node.left, Val::Px(x));
        set(&mut node.top, Val::Px(y));
        set(&mut node.width, Val::Px(w));
        set(&mut node.height, Val::Px(h));
    }
}

/// Frame rate and ping in the corner, when switched on in Settings.
fn net_stats(time: Res<Time<Real>>, settings: Res<crate::settings::Settings>, role: Res<NetRole>, ping: Res<oz_sim::net::Ping>, mut text: Query<&mut Text, With<NetStats>>, mut fps: Local<f32>) {
    let Ok(mut text) = text.single_mut() else { return };
    let dt = time.delta_secs().max(1e-4);
    *fps += (1.0 / dt - *fps) * 0.05;
    let wanted = if !settings.net_stats {
        String::new()
    } else if *role == NetRole::Client {
        format!("{:.0} FPS\n{} MS", *fps, ping.0)
    } else {
        format!("{:.0} FPS", *fps)
    };
    if text.0 != wanted {
        text.0 = wanted;
    }
}
