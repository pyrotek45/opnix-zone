//! Small things that make the game easier to read:
//!
//! * **Limb health:** a little robot by the health bar - head, body, arms and legs
//!   tinted from green through amber to red as they take damage, dark once shot off.
//! * **Hit direction:** red marks round the crosshair pointing at whoever just hit you
//!   (or the blast that did), fading over a second.
//! * **Ammo warning:** the round counter turns amber when the magazine runs low and red
//!   when it is empty.
//! * **Aim assist (controller only):** with the crosshair on or just beside a visible enemy
//!   the stick turns 40% slower, and while you steer or move the aim drifts gently with
//!   them. Modest by design: it helps a thumbstick track, it never aims for you.

use crate::{ClientSet, ui::Screen, view::WorldCamera};
use bevy::prelude::*;
use oz_sim::prelude::*;

/// One block of the limb figure: 0 head, 1 body, 2 left arm, 3 right arm, 4 left leg, 5 right leg.
#[derive(Component)]
struct Doll(usize);

#[derive(Component)]
struct DollRoot;

/// One of the hit-direction marks round the crosshair.
#[derive(Component)]
struct HitMark(usize);

const MARKS: usize = 6;
/// How far from the centre of the screen the marks sit (pixels).
const MARK_RADIUS: f32 = 70.0;

/// Recent hits on the local player: direction (world, flat) and seconds left.
#[derive(Resource, Default)]
struct Hits(Vec<(Vec3, f32)>);

pub struct QolPlugin;

impl Plugin for QolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hits>()
            .add_systems(OnEnter(MatchPhase::Active), (spawn, |mut hits: ResMut<Hits>| hits.0.clear()))
            .add_systems(Update, (doll, (track_hits, show_hits).chain()).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, aim_assist.in_set(ClientSet::Input).after(crate::input::ReadDevices).before(crate::input::DrivePlayer).run_if(in_state(MatchPhase::Active)));
    }
}

fn spawn(mut commands: Commands) {
    // The figure: just right of the health panel, bottom left.
    let block = |left: f32, top: f32, width: f32, height: f32| Node { position_type: PositionType::Absolute, left: Val::Px(left), top: Val::Px(top), width: Val::Px(width), height: Val::Px(height), ..default() };
    commands
        .spawn((
            MatchEntity,
            DollRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(292.0),
                bottom: Val::Px(26.0),
                width: Val::Px(34.0),
                height: Val::Px(52.0),
                ..default()
            },
            GlobalZIndex(-40),
            Visibility::Hidden,
        ))
        .with_children(|doll| {
            let parts = [block(11.0, 0.0, 12.0, 10.0), block(9.0, 12.0, 16.0, 18.0), block(1.0, 12.0, 6.0, 17.0), block(27.0, 12.0, 6.0, 17.0), block(9.0, 32.0, 7.0, 20.0), block(18.0, 32.0, 7.0, 20.0)];
            for (i, node) in parts.into_iter().enumerate() {
                doll.spawn((Doll(i), node, BackgroundColor(Color::srgb(0.3, 0.9, 0.45))));
            }
        });
    for i in 0..MARKS {
        commands.spawn((
            MatchEntity,
            HitMark(i),
            Node { position_type: PositionType::Absolute, width: Val::Px(10.0), height: Val::Px(10.0), ..default() },
            BackgroundColor(Color::srgba(1.0, 0.15, 0.1, 0.0)),
            BorderRadius::all(Val::Px(5.0)),
            GlobalZIndex(-39),
            Visibility::Hidden,
        ));
    }
}

/// Green when whole, amber when hurt, red when nearly gone, dark when lost.
fn shade(fraction: f32, lost: bool) -> Color {
    if lost {
        return Color::srgba(0.12, 0.12, 0.14, 0.9);
    }
    let f = fraction.clamp(0.0, 1.0);
    if f > 0.6 {
        Color::srgb(0.3, 0.9, 0.45)
    } else if f > 0.3 {
        Color::srgb(1.0, 0.75, 0.2)
    } else {
        Color::srgb(1.0, 0.25, 0.15)
    }
}

fn doll(
    screen: Res<Screen>,
    players: Query<(&Player, &Health), With<LocalPlayer>>,
    mut root: Query<&mut Visibility, With<DollRoot>>,
    mut parts: Query<(&Doll, &mut BackgroundColor)>,
) {
    let Ok((player, health)) = players.single() else { return };
    let shown = matches!(*screen, Screen::None | Screen::Chat) && health.alive();
    if let Ok(mut visibility) = root.single_mut() {
        let wanted = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    if !shown {
        return;
    }
    let body = health.hp / health.max.max(1.0);
    for (part, mut color) in &mut parts {
        let wanted = match part.0 {
            0 | 1 => shade(body, false),
            limb => {
                let slot = limb - 2;
                shade(player.limb_hp[slot] / oz_sim::player::LIMB_HP, player.lost & (1 << slot) != 0)
            }
        };
        if color.0 != wanted {
            color.0 = wanted;
        }
    }
}

/// Works out where hits on us came from: the shot that landed by us this frame, or the
/// blast that went off nearby.
fn track_hits(
    time: Res<Time>,
    mut events: EventReader<FxEvent>,
    mut hits: ResMut<Hits>,
    players: Query<(&Player, &Transform), With<LocalPlayer>>,
    mut recent: Local<Vec<(Vec3, Vec3, f32, u8)>>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    for hit in hits.0.iter_mut() {
        hit.1 -= dt;
    }
    hits.0.retain(|h| h.1 > 0.0);
    recent.retain(|r| now - r.2 < 0.3);
    let Ok((me, transform)) = players.single() else {
        events.clear();
        return;
    };
    let chest = transform.translation + Vec3::Y * 1.0;
    let mut hurt = false;
    for event in events.read() {
        match event {
            FxEvent::Shot { by, from, to, .. } if *by != me.id => recent.push((*from, *to, now, *by)),
            FxEvent::Explosion { at, radius } if at.distance(chest) < radius * 1.5 => recent.push((*at, chest, now, 255)),
            FxEvent::Hurt { player } if *player == me.id => hurt = true,
            _ => {}
        }
    }
    if !hurt {
        return;
    }
    // The shot whose end landed closest to us is the one that hurt.
    let source = recent.iter().filter(|(_, to, ..)| to.distance(chest) < 2.5).min_by(|a, b| a.1.distance(chest).total_cmp(&b.1.distance(chest))).map(|(from, ..)| *from);
    if let Some(from) = source {
        let dir = (from - chest).with_y(0.0).normalize_or_zero();
        if dir != Vec3::ZERO {
            hits.0.push((dir, 1.2));
            if hits.0.len() > 3 {
                hits.0.remove(0);
            }
        }
    }
}

fn show_hits(hits: Res<Hits>, windows: Query<&Window>, camera: Query<&GlobalTransform, With<WorldCamera>>, mut marks: Query<(&HitMark, &mut Node, &mut BackgroundColor, &mut Visibility)>) {
    let (Ok(window), Ok(camera)) = (windows.single(), camera.single()) else { return };
    let centre = Vec2::new(window.width(), window.height()) * 0.5;
    let forward = camera.forward().with_y(0.0).normalize_or_zero();
    let right = camera.right().with_y(0.0).normalize_or_zero();
    for (mark, mut node, mut color, mut visibility) in &mut marks {
        // Two marks per hit, side by side along the arc.
        let Some((dir, left)) = hits.0.get(mark.0 / 2).copied() else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        let angle = dir.dot(right).atan2(dir.dot(forward)) + if mark.0 % 2 == 0 { -0.12 } else { 0.12 };
        // Screen up is "in front of you".
        let at = centre + Vec2::new(angle.sin(), -angle.cos()) * MARK_RADIUS;
        node.left = Val::Px(at.x - 5.0);
        node.top = Val::Px(at.y - 5.0);
        color.0 = Color::srgba(1.0, 0.15, 0.1, (left / 1.2).clamp(0.0, 1.0) * 0.9);
        if *visibility != Visibility::Inherited {
            *visibility = Visibility::Inherited;
        }
    }
}

/// Look speed with the crosshair over a target, and how strongly the aim follows one.
const ASSIST_SLOWDOWN: f32 = 0.6;
const ASSIST_PULL: f32 = 2.2;

#[allow(clippy::too_many_arguments)]
fn aim_assist(
    time: Res<Time>,
    screen: Res<Screen>,
    rules: Res<Rules>,
    statics: Res<oz_sim::session::Statics>,
    mut controls: ResMut<crate::input::Controls>,
    settings: Res<crate::settings::Settings>,
    me: Query<(&Player, &Transform, &PlayerInput, &Health), With<LocalPlayer>>,
    others: Query<(&Player, &Transform, &PlayerInput, &Health), Without<LocalPlayer>>,
    enemies: Query<(&Transform, &Health), (With<oz_sim::enemy::Enemy>, Without<Player>)>,
) {
    if !controls.gamepad || !settings.aim_assist || *screen != Screen::None {
        return;
    }
    let Ok((me, at, input, health)) = me.single() else { return };
    if !health.alive() {
        return;
    }
    let eye = at.translation + Vec3::Y * input.eye();
    let look = input.look() * Vec3::NEG_Z;
    // Chest heights of everyone worth aiming at.
    let mut targets: Vec<Vec3> = others
        .iter()
        .filter(|(p, _, _, h)| rules.pvp && h.alive() && p.team != me.team)
        .map(|(_, t, i, _)| t.translation + Vec3::Y * (stance::eye(i.stance) - 0.35))
        .collect();
    targets.extend(enemies.iter().filter(|(_, h)| h.alive()).map(|(t, _)| t.translation + Vec3::Y * 1.1));
    // The best target: inside the assist cone (about a body's width round it, a little
    // wider down the sights), in sight, nearest the crosshair.
    let cone = if controls.aim { 1.7 } else { 1.2 };
    let best = targets
        .into_iter()
        .filter_map(|target| {
            let to = target - eye;
            let distance = to.length();
            if !(1.0..90.0).contains(&distance) {
                return None;
            }
            let dir = to / distance;
            let off = look.angle_between(dir);
            let radius = (0.55 * cone / distance).atan();
            (off < radius && statics.raycast(eye, dir, distance - 0.4).is_none()).then_some((off / radius, dir))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    let Some((_, dir)) = best else { return };
    // Slowdown: finer control right where it matters.
    controls.look *= ASSIST_SLOWDOWN;
    // Pull: only while the player is steering or moving, never on its own.
    if controls.look.length() > 1e-4 || controls.movement.length() > 0.2 {
        let want_yaw = f32::atan2(-dir.x, -dir.z);
        let want_pitch = dir.y.clamp(-1.0, 1.0).asin();
        let yaw_err = (want_yaw - input.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let pitch_err = want_pitch - input.pitch;
        let k = (ASSIST_PULL * time.delta_secs()).min(1.0);
        // The input layer subtracts `look` from yaw and pitch.
        controls.look += Vec2::new(-yaw_err * k, -pitch_err * k);
    }
}
