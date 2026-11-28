//! Horde mode on screen: the husks' shambling rigs, the map's furniture (chalk wall buys,
//! perk machines, the salvage crate, the reforge, the generator, arc grates, relay beacons,
//! doors and boards), power-ups,
//! and the HUD - the round in the corner, everyone's points with +10 popups, perk badges,
//! what Use would do here, and the down / revive bars.
//!
//! Everything is read from `MatchState::horde` and the map's `HordeLayout`, so it is the same
//! on a host and on a client. Use is sent as `PlayerCommand::Mode(act::USE)` on the press and
//! `act::HOLD` while held.

use crate::{
    ClientSet,
    art::{Art, Baked},
    bindings::Action,
    input::Controls,
    settings::Settings,
    ui::{self, Screen},
    view::WorldCamera,
};
use bevy::{pbr::NotShadowCaster, prelude::*};
use oz_core::{
    arsenal::assemble,
    worldgen::{CHALK_FRAME, chalk_fit},
};
use oz_sim::{
    prelude::*,
    zombies::{self, Interact, act, crate_phase, drop, forge_phase, kind, perk},
};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

pub struct HordeHudPlugin;

impl Plugin for HordeHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Prompt>()
            .add_systems(OnEnter(MatchPhase::Active), (dress_map, spawn_hud).run_if(zombies::horde))
            .add_systems(Update, demo.in_set(ClientSet::Input).after(crate::input::DrivePlayer).run_if(in_state(MatchPhase::Active)).run_if(zombies::horde))
            .add_systems(Update, send_use.in_set(ClientSet::Input).after(crate::input::ReadDevices).run_if(in_state(MatchPhase::Active)).run_if(zombies::horde))
            .add_systems(Update, (attach_husks, animate_husks, update_map, update_extras, update_drops, update_hud, float_popups).chain().in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)).run_if(zombies::horde));
    }
}

/// Developer photo spot: `OPNIX_HORDE_DEMO=x,z,yaw_degrees[,pitch_degrees][,open]` keeps the
/// local player standing there looking that way (`open` also opens every door, turns the power
/// on and hands out points). Handy for screenshots.
fn demo(mut state: ResMut<MatchState>, mut players: Query<(&mut Transform, &mut PlayerInput), With<LocalPlayer>>, map: Res<Map>, mut done: Local<bool>) {
    let Ok(spec) = std::env::var("OPNIX_HORDE_DEMO") else { return };
    let parts: Vec<&str> = spec.split(',').collect();
    let num = |i: usize| parts.get(i).and_then(|v| v.trim().parse::<f32>().ok());
    let (Some(x), Some(z), Some(yaw)) = (num(0), num(1), num(2)) else { return };
    let Ok((mut transform, mut input)) = players.single_mut() else { return };
    transform.translation = Vec3::new(x, map.horde.as_ref().map_or(0.0, |l| l.ground), z);
    input.yaw = yaw.to_radians();
    input.pitch = num(3).unwrap_or(0.0).to_radians();
    if !*done && !state.horde.survivors.is_empty() && parts.iter().any(|p| p.trim() == "open") {
        *done = true;
        let h = &mut state.horde;
        h.doors = u64::MAX;
        h.power = true;
        for s in &mut h.survivors {
            s.points = 12_340;
        }
    }
}

/// What Use would do right now for the local player (shared by the sender and the HUD).
#[derive(Resource, Default)]
pub struct Prompt(pub Option<Interact>);

fn glow(paint: Paint) -> Paint {
    match paint {
        Paint::Gold => Paint::Glow,
        Paint::Coral => Paint::Spark,
        other => other,
    }
}

fn piece(commands: &mut Commands, art: &Art, parent: Entity, shape: Shape, paint: Paint, at: Vec3, size: Vec3, rot: Quat) -> Entity {
    let mut e = commands.spawn((Mesh3d(art.primitive(shape)), MeshMaterial3d(art.paint(paint)), Transform { translation: at, rotation: rot, scale: size }, ChildOf(parent)));
    if paint.unlit() {
        e.insert(NotShadowCaster);
    }
    e.id()
}

fn baked(commands: &mut Commands, art: &Art, parent: Entity, baked: &Baked, transform: Transform) -> Entity {
    let root = commands.spawn((transform, Visibility::Inherited, ChildOf(parent))).id();
    if let Some(mesh) = &baked.lit {
        commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(art.lit.clone()), ChildOf(root)));
    }
    if let Some(mesh) = &baked.unlit {
        commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(art.unlit.clone()), NotShadowCaster, ChildOf(root)));
    }
    root
}

/// A gun's model, all in one paint (chalk on the walls).
fn chalk(model: &BlockModel, paint: Paint) -> BlockModel {
    let mut m = model.clone();
    for part in &mut m.parts {
        part.paint = paint;
    }
    for hull in &mut m.hulls {
        hull.paint = paint;
    }
    m
}

#[derive(Component)]
struct DoorLook(usize);
#[derive(Component)]
struct Board(usize, u8);
#[derive(Component)]
struct PerkLight(u8);
#[derive(Component)]
struct PowerLever;
#[derive(Component)]
struct CrateLook;
#[derive(Component)]
struct CrateGun(u8);
#[derive(Component)]
struct ForgeGun(u8);
#[derive(Component)]
struct ForgeCore;
#[derive(Component)]
struct DropLook(usize);
/// An arc grate's live bars (`.1` false) or its panel lamp (`.1` true).
#[derive(Component)]
struct TrapLook(usize, bool);
#[derive(Component)]
struct RelayLamp(usize);

fn machine_center(m: &oz_core::worldgen::HordeMachine) -> Vec3 {
    m.pos - Vec3::new(m.facing.x, 0.0, m.facing.y) * 0.95
}

fn facing_rotation(facing: Vec2) -> Quat {
    // Model fronts face -Z; turn them to face `facing`.
    Quat::from_rotation_y(f32::atan2(-facing.x, -facing.y))
}

fn dress_map(mut commands: Commands, map: Res<Map>, content: Res<Content>, art: Res<Art>, mut meshes: ResMut<Assets<Mesh>>) {
    let Some(layout) = map.horde.as_deref() else { return };
    let guns: Vec<Baked> = content.weapons.iter().map(|w| art.bake(&assemble(w, &w.defaults), &mut meshes)).collect();
    // Wall buys: the gun's outline in chalk, centred in its chalk frame and sized to fit it
    // (long guns shrink, small ones grow a little). The frame's local Z runs along the wall,
    // X out of it.
    for buy in &layout.buys {
        let Some(def) = content.weapons.get(buy.weapon as usize) else { continue };
        let model = chalk(&assemble(def, &def.defaults), Paint::White);
        let (scale, mid) = model.bounds().map_or((1.0, Vec3::ZERO), |(lo, hi)| chalk_fit(lo, hi));
        let outline = art.bake(&model, &mut meshes);
        let n = Vec3::new(buy.normal.x, 0.0, buy.normal.y);
        let rot = Quat::from_rotation_y(f32::atan2(-n.z, n.x));
        let root = commands.spawn((MatchEntity, Transform::from_translation(buy.pos + n * 0.03).with_rotation(rot), Visibility::Inherited)).id();
        // Flattened against the wall: barely any depth.
        let flat = 0.08;
        let size = Vec3::new(flat, scale, scale);
        baked(&mut commands, &art, root, &outline, Transform { translation: -mid * size, scale: size, ..default() });
        let (w, h) = (CHALK_FRAME.x, CHALK_FRAME.y);
        for (at, size) in [
            (Vec3::new(0.0, h * 0.5, 0.0), Vec3::new(0.02, 0.03, w + 0.03)),
            (Vec3::new(0.0, -h * 0.5, 0.0), Vec3::new(0.02, 0.03, w + 0.03)),
            (Vec3::new(0.0, 0.0, w * 0.5), Vec3::new(0.02, h + 0.03, 0.03)),
            (Vec3::new(0.0, 0.0, -w * 0.5), Vec3::new(0.02, h + 0.03, 0.03)),
        ] {
            piece(&mut commands, &art, root, Shape::Cube, Paint::Bone, at, size, Quat::IDENTITY);
        }
    }
    // Perk machines: a lit front, a sign on top, an emblem.
    for machine in &layout.perks {
        let root = commands.spawn((MatchEntity, Transform::from_translation(machine_center(machine)).with_rotation(facing_rotation(machine.facing)), Visibility::Inherited)).id();
        let color = perk::COLORS[machine.kind as usize];
        piece(&mut commands, &art, root, Shape::Cube, color, Vec3::new(0.0, 1.1, -0.43), Vec3::new(1.05, 1.9, 0.04), Quat::IDENTITY);
        let lit = piece(&mut commands, &art, root, Shape::Cube, glow(color), Vec3::new(0.0, 1.35, -0.46), Vec3::new(0.8, 0.9, 0.03), Quat::IDENTITY);
        commands.entity(lit).insert(PerkLight(machine.kind));
        let sign = piece(&mut commands, &art, root, Shape::Cube, glow(color), Vec3::new(0.0, 2.38, -0.2), Vec3::new(1.15, 0.28, 0.5), Quat::IDENTITY);
        commands.entity(sign).insert(PerkLight(machine.kind));
        let emblem = [Shape::Shard, Shape::Cylinder, Shape::Cone, Shape::Cube, Shape::Shard, Shape::Cylinder][machine.kind as usize % 6];
        piece(&mut commands, &art, root, emblem, Paint::White, Vec3::new(0.0, 1.35, -0.5), Vec3::splat(0.32), Quat::from_rotation_z(machine.kind as f32 * 0.4));
        piece(&mut commands, &art, root, Shape::Cube, Paint::Dark, Vec3::new(0.0, 0.5, -0.47), Vec3::new(0.5, 0.18, 0.04), Quat::IDENTITY);
    }
    // The reforge: an anvil press over a glowing core.
    if let Some(forge) = layout.forge {
        let root = commands.spawn((MatchEntity, Transform::from_translation(machine_center(&forge)).with_rotation(facing_rotation(forge.facing)), Visibility::Inherited)).id();
        piece(&mut commands, &art, root, Shape::Cube, Paint::Iron, Vec3::new(0.0, 1.75, 0.0), Vec3::new(1.3, 0.3, 0.95), Quat::IDENTITY);
        piece(&mut commands, &art, root, Shape::Cube, Paint::Steel, Vec3::new(0.0, 2.6, 0.15), Vec3::new(0.25, 1.4, 0.25), Quat::IDENTITY);
        piece(&mut commands, &art, root, Shape::Cube, Paint::Dark, Vec3::new(0.0, 3.3, 0.0), Vec3::new(1.4, 0.3, 0.7), Quat::IDENTITY);
        let core = piece(&mut commands, &art, root, Shape::Shard, Paint::Violet, Vec3::new(0.0, 1.0, -0.46), Vec3::splat(0.4), Quat::IDENTITY);
        commands.entity(core).insert(ForgeCore);
        for (i, gun) in guns.iter().enumerate() {
            let g = baked(&mut commands, &art, root, gun, Transform::from_xyz(0.0, 2.15, -0.1).with_rotation(Quat::from_rotation_y(FRAC_PI_2)));
            commands.entity(g).insert((ForgeGun(i as u8), Visibility::Hidden));
        }
    }
    // The power switch: a box on the wall with a big lever and a lamp.
    if let Some(power) = layout.power {
        let at = power.pos - Vec3::new(power.facing.x, 0.0, power.facing.y) * 1.3;
        let root = commands.spawn((MatchEntity, Transform::from_translation(at).with_rotation(facing_rotation(power.facing)), Visibility::Inherited)).id();
        piece(&mut commands, &art, root, Shape::Cube, Paint::Steel, Vec3::new(0.0, 1.4, 0.0), Vec3::new(0.8, 1.0, 0.25), Quat::IDENTITY);
        piece(&mut commands, &art, root, Shape::Cube, Paint::Gold, Vec3::new(0.0, 1.4, -0.13), Vec3::new(0.6, 0.06, 0.02), Quat::IDENTITY);
        let lever = commands.spawn((PowerLever, Transform::from_xyz(0.0, 1.4, -0.15).with_rotation(Quat::from_rotation_x(0.9)), Visibility::Inherited, ChildOf(root))).id();
        piece(&mut commands, &art, lever, Shape::Cube, Paint::Dark, Vec3::new(0.0, 0.25, 0.0), Vec3::new(0.08, 0.5, 0.08), Quat::IDENTITY);
        piece(&mut commands, &art, lever, Shape::Cube, Paint::Red, Vec3::new(0.0, 0.52, 0.0), Vec3::new(0.2, 0.1, 0.1), Quat::IDENTITY);
    }
    // Arc grates: steel bars across the floor that light up while live, and a switch panel
    // with a lamp (dark without power, green ready, amber live, red cooling).
    for (i, trap) in layout.traps.iter().enumerate() {
        let (x0, x1, z0, z1) = trap.floor;
        let y = layout.ground + 0.04;
        let root = commands.spawn((MatchEntity, Transform::from_xyz((x0 + x1) * 0.5, y, (z0 + z1) * 0.5), Visibility::Inherited)).id();
        let (w, d) = (x1 - x0, z1 - z0);
        let along_x = w >= d;
        let (long, short) = if along_x { (w, d) } else { (d, w) };
        let bars = (long / 0.5) as usize;
        for k in 0..bars {
            let u = -long * 0.5 + (k as f32 + 0.5) * long / bars as f32;
            let at = if along_x { Vec3::new(u, 0.0, 0.0) } else { Vec3::new(0.0, 0.0, u) };
            let size = if along_x { Vec3::new(0.1, 0.05, short - 0.1) } else { Vec3::new(short - 0.1, 0.05, 0.1) };
            piece(&mut commands, &art, root, Shape::Cube, Paint::Steel, at, size, Quat::IDENTITY);
            if k % 2 == 0 {
                let glow = piece(&mut commands, &art, root, Shape::Cube, Paint::Aether, at + Vec3::Y * 0.03, size * Vec3::new(0.6, 1.0, 1.0).min(Vec3::ONE) + Vec3::Y * 0.01, Quat::IDENTITY);
                commands.entity(glow).insert((TrapLook(i, false), Visibility::Hidden));
            }
        }
        let panel = trap.panel;
        let at = panel.pos - Vec3::new(panel.facing.x, 0.0, panel.facing.y) * 1.3;
        let root = commands.spawn((MatchEntity, Transform::from_translation(at).with_rotation(facing_rotation(panel.facing)), Visibility::Inherited)).id();
        piece(&mut commands, &art, root, Shape::Cube, Paint::Dark, Vec3::new(0.0, 1.35, 0.0), Vec3::new(0.7, 0.9, 0.22), Quat::IDENTITY);
        piece(&mut commands, &art, root, Shape::Cube, Paint::Gold, Vec3::new(0.0, 1.35, -0.12), Vec3::new(0.62, 0.08, 0.02), Quat::IDENTITY);
        piece(&mut commands, &art, root, Shape::Shard, Paint::Gold, Vec3::new(0.0, 1.62, -0.12), Vec3::new(0.18, 0.18, 0.03), Quat::IDENTITY);
        let lamp = piece(&mut commands, &art, root, Shape::Cube, Paint::Dark, Vec3::new(0.0, 1.06, -0.13), Vec3::new(0.3, 0.14, 0.03), Quat::IDENTITY);
        commands.entity(lamp).insert(TrapLook(i, true));
        // A cable from the panel down to the floor.
        piece(&mut commands, &art, root, Shape::Cube, Paint::Dark, Vec3::new(0.25, 0.45, 0.0), Vec3::new(0.06, 0.9, 0.06), Quat::IDENTITY);
    }
    // Relay beacons: a mast on the transmitter with a lamp that lights when switched on.
    for (i, relay) in layout.relays.iter().enumerate() {
        let root = commands.spawn((MatchEntity, Transform::from_translation(machine_center(relay)).with_rotation(facing_rotation(relay.facing)), Visibility::Inherited)).id();
        piece(&mut commands, &art, root, Shape::Cube, Paint::Steel, Vec3::new(0.0, 0.85, -0.43), Vec3::new(0.9, 0.5, 0.03), Quat::IDENTITY);
        piece(&mut commands, &art, root, Shape::Cube, Paint::Screen, Vec3::new(-0.2, 0.9, -0.45), Vec3::new(0.35, 0.22, 0.02), Quat::IDENTITY);
        piece(&mut commands, &art, root, Shape::Cylinder, Paint::Iron, Vec3::new(0.25, 1.9, 0.0), Vec3::new(0.06, 1.4, 0.06), Quat::IDENTITY);
        piece(&mut commands, &art, root, Shape::Cube, Paint::Iron, Vec3::new(0.25, 2.3, 0.0), Vec3::new(0.6, 0.04, 0.04), Quat::IDENTITY);
        let lamp = piece(&mut commands, &art, root, Shape::Cube, Paint::Red, Vec3::new(0.25, 2.65, 0.0), Vec3::splat(0.2), Quat::from_rotation_y(0.785));
        commands.entity(lamp).insert(RelayLamp(i));
    }
    // The crate: one, moved between its spots.
    let crate_root = commands.spawn((MatchEntity, CrateLook, Transform::default(), Visibility::Hidden)).id();
    piece(&mut commands, &art, crate_root, Shape::Cube, Paint::Wood, Vec3::new(0.0, 0.45, 0.0), Vec3::new(1.5, 0.9, 0.8), Quat::IDENTITY);
    for x in [-0.72, 0.72] {
        piece(&mut commands, &art, crate_root, Shape::Cube, Paint::Dark, Vec3::new(x, 0.45, 0.0), Vec3::new(0.08, 0.95, 0.85), Quat::IDENTITY);
    }
    piece(&mut commands, &art, crate_root, Shape::Cube, Paint::Glow, Vec3::new(0.0, 0.92, 0.0), Vec3::new(1.4, 0.04, 0.7), Quat::IDENTITY);
    piece(&mut commands, &art, crate_root, Shape::Shard, Paint::Glow, Vec3::new(0.0, 0.45, -0.41), Vec3::new(0.25, 0.25, 0.04), Quat::IDENTITY);
    piece(&mut commands, &art, crate_root, Shape::Cylinder, Paint::Aether, Vec3::new(0.0, 6.0, 0.0), Vec3::new(0.12, 10.0, 0.12), Quat::IDENTITY);
    for (i, gun) in guns.iter().enumerate() {
        let g = baked(&mut commands, &art, crate_root, gun, Transform::from_xyz(0.0, 1.3, 0.0).with_rotation(Quat::from_rotation_y(FRAC_PI_2)));
        commands.entity(g).insert((CrateGun(i as u8), Visibility::Hidden));
    }
    // Doors: a steel shutter with hazard stripes, or a heap of rubble.
    for (i, door) in layout.doors.iter().enumerate() {
        let c = door.aabb.center;
        let h = door.aabb.half;
        let root = commands.spawn((MatchEntity, DoorLook(i), Transform::from_translation(c), Visibility::Inherited)).id();
        if door.debris {
            let mut k = 0;
            for (dx, dy, s) in [(-0.7, -1.0, 0.9), (0.6, -1.05, 0.8), (0.0, -0.5, 0.9), (-0.5, 0.1, 0.6), (0.5, 0.0, 0.7), (0.0, 0.7, 0.6)] {
                let along = if door.along_x { Vec3::X } else { Vec3::Z };
                k += 1;
                piece(&mut commands, &art, root, if k % 2 == 0 { Shape::Shard } else { Shape::Cube }, if k % 3 == 0 { Paint::Wood } else { Paint::Rock }, along * dx + Vec3::Y * dy, Vec3::new(s, s * 0.8, s), Quat::from_rotation_y(k as f32));
            }
            for k in 0..3 {
                let along = if door.along_x { Vec3::X } else { Vec3::Z };
                piece(&mut commands, &art, root, Shape::Cube, Paint::Wood, along * (k as f32 - 1.0) * 0.4 + Vec3::Y * (0.2 - k as f32 * 0.3), if door.along_x { Vec3::new(2.2, 0.15, 0.15) } else { Vec3::new(0.15, 0.15, 2.2) }, Quat::from_rotation_y(0.3 * k as f32) * Quat::from_rotation_z(0.25));
            }
        } else {
            piece(&mut commands, &art, root, Shape::Cube, Paint::Steel, Vec3::ZERO, h * 2.0 - Vec3::new(0.0, 0.0, 0.0), Quat::IDENTITY);
            for k in 0..4 {
                let y = -h.y + 0.4 + k as f32 * 0.6;
                let size = if door.along_x { Vec3::new(h.x * 2.0, 0.08, h.z * 2.0 + 0.02) } else { Vec3::new(h.x * 2.0 + 0.02, 0.08, h.z * 2.0) };
                piece(&mut commands, &art, root, Shape::Cube, if k % 2 == 0 { Paint::Gold } else { Paint::Dark }, Vec3::Y * y, size, Quat::IDENTITY);
            }
            piece(&mut commands, &art, root, Shape::Cube, Paint::Red, Vec3::Y * (h.y - 0.25), Vec3::new(0.2, 0.2, 0.2) + if door.along_x { Vec3::new(0.0, 0.0, h.z * 2.0) } else { Vec3::new(h.x * 2.0, 0.0, 0.0) }, Quat::IDENTITY);
        }
    }
    // Boards over the windows: six each, nailed across at odd angles.
    for (i, window) in layout.windows.iter().enumerate() {
        let n = Vec3::new(window.out.x, 0.0, window.out.y);
        let root = commands.spawn((MatchEntity, Transform::from_translation(window.center + n * 0.28).with_rotation(Quat::from_rotation_y(f32::atan2(-n.x, -n.z))), Visibility::Inherited)).id();
        for k in 0..zombies::PLANKS {
            let y = oz_core::worldgen::HORDE_SILL + 0.15 + k as f32 * 0.22;
            let tilt = if k % 2 == 0 { 0.18 } else { -0.14 } + (i as f32 * 1.7 + k as f32).sin() * 0.06;
            let b = piece(&mut commands, &art, root, Shape::Cube, if k % 3 == 1 { Paint::Rust } else { Paint::Wood }, Vec3::new((k as f32 * 0.7).sin() * 0.06, y, 0.0), Vec3::new(1.85, 0.16, 0.06), Quat::from_rotation_z(tilt));
            commands.entity(b).insert(Board(i, k));
        }
    }
}

fn update_map(
    time: Res<Time>,
    map: Res<Map>,
    state: Res<MatchState>,
    mut doors: Query<(&DoorLook, &mut Visibility), (Without<Board>, Without<CrateLook>, Without<CrateGun>, Without<ForgeGun>)>,
    mut boards: Query<(&Board, &mut Visibility), (Without<DoorLook>, Without<CrateLook>, Without<CrateGun>, Without<ForgeGun>)>,
    mut crate_look: Query<(&mut Transform, &mut Visibility), (With<CrateLook>, Without<DoorLook>, Without<Board>, Without<CrateGun>, Without<ForgeGun>, Without<PowerLever>, Without<ForgeCore>)>,
    mut crate_guns: Query<(&CrateGun, &mut Visibility, &mut Transform), (Without<DoorLook>, Without<Board>, Without<CrateLook>, Without<ForgeGun>, Without<PowerLever>, Without<ForgeCore>)>,
    mut forge_guns: Query<(&ForgeGun, &mut Visibility, &mut Transform), (Without<DoorLook>, Without<Board>, Without<CrateLook>, Without<CrateGun>, Without<PowerLever>, Without<ForgeCore>)>,
    mut lever: Query<&mut Transform, (With<PowerLever>, Without<CrateLook>, Without<CrateGun>, Without<ForgeGun>, Without<ForgeCore>)>,
    mut core: Query<&mut Transform, (With<ForgeCore>, Without<PowerLever>, Without<CrateLook>, Without<CrateGun>, Without<ForgeGun>)>,
    mut lights: Query<(&PerkLight, &mut MeshMaterial3d<StandardMaterial>)>,
    art: Res<Art>,
    content: Res<Content>,
) {
    let Some(layout) = map.horde.as_deref() else { return };
    let h = &state.horde;
    let t = time.elapsed_secs();
    let set = |v: &mut Visibility, on: bool| {
        let want = if on { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    };
    for (door, mut v) in &mut doors {
        set(&mut v, !h.door_open(door.0));
    }
    for (board, mut v) in &mut boards {
        set(&mut v, h.planks.get(board.0).is_some_and(|p| board.1 < *p));
    }
    for (light, mut material) in &mut lights {
        let on = h.power || light.0 == perk::PATCH_UP;
        let want = art.paint(if on { glow(perk::COLORS[light.0 as usize]) } else { Paint::Dark });
        if material.0 != want {
            material.0 = want;
        }
    }
    for mut lever in &mut lever {
        lever.rotation = Quat::from_rotation_x(if h.power { -0.9 } else { 0.9 - 1.2 * h.charge });
    }
    for mut core in &mut core {
        let working = h.forge_phase == forge_phase::WORKING;
        let s = if h.power { 0.4 + if working { 0.12 * (t * 14.0).sin() } else { 0.04 * (t * 3.0).sin() } } else { 0.15 };
        core.scale = Vec3::splat(s);
        core.rotation = Quat::from_rotation_y(t * if working { 6.0 } else { 0.8 });
    }
    // The crate at its spot; while it rolls, guns flicker over it.
    if let Ok((mut transform, mut v)) = crate_look.single_mut() {
        let spot = layout.crates.get(h.crate_at as usize);
        set(&mut v, spot.is_some() && h.crate_phase != crate_phase::LEAVING);
        if let Some(spot) = spot {
            transform.translation = machine_center(spot);
            transform.rotation = facing_rotation(spot.facing);
        }
    }
    let half = content.weapons.len() / 2;
    let flicker = ((t * 9.0) as usize * 7) % half.max(1);
    for (gun, mut v, mut transform) in &mut crate_guns {
        let show = match h.crate_phase {
            crate_phase::ROLLING => gun.0 as usize == flicker && gun.0 != 0,
            crate_phase::OFFER => gun.0 == h.crate_weapon,
            _ => false,
        };
        set(&mut v, show);
        if show {
            let rise = if h.crate_phase == crate_phase::ROLLING { (4.2 - h.crate_timer).clamp(0.0, 4.2) * 0.12 } else { 0.5 };
            transform.translation.y = 1.0 + rise;
            transform.rotation = Quat::from_rotation_y(FRAC_PI_2 + if h.crate_phase == crate_phase::OFFER { (t * 1.5).sin() * 0.4 } else { 0.0 });
        }
    }
    for (gun, mut v, mut transform) in &mut forge_guns {
        let show = h.forge_phase == forge_phase::OFFER && gun.0 == h.forge_weapon;
        set(&mut v, show);
        if show {
            transform.rotation = Quat::from_rotation_y(FRAC_PI_2 + t * 1.2);
        }
    }
}

/// Arc grates and relay beacons: lit by the state.
fn update_extras(
    time: Res<Time>,
    state: Res<MatchState>,
    art: Res<Art>,
    mut traps: Query<(&TrapLook, &mut MeshMaterial3d<StandardMaterial>, &mut Visibility)>,
    mut relays: Query<(&RelayLamp, &mut MeshMaterial3d<StandardMaterial>), Without<TrapLook>>,
) {
    let h = &state.horde;
    let t = time.elapsed_secs();
    for (look, mut material, mut v) in &mut traps {
        let (running, left) = h.trap(look.0);
        let paint = if look.1 {
            if !h.power {
                Paint::Dark
            } else if running {
                Paint::Glow
            } else if left > 0.0 {
                Paint::Red
            } else {
                Paint::Green
            }
        } else if (t * 17.0 + look.0 as f32).sin() > 0.0 {
            Paint::Aether
        } else {
            Paint::White
        };
        let want = art.paint(paint);
        if material.0 != want {
            material.0 = want;
        }
        if !look.1 {
            let show = if running { Visibility::Inherited } else { Visibility::Hidden };
            if *v != show {
                *v = show;
            }
        }
    }
    for (lamp, mut material) in &mut relays {
        let on = h.relay_on(lamp.0);
        let blink = (t * 2.5 + lamp.0 as f32).sin() > 0.0;
        let want = art.paint(if on { Paint::Aether } else if blink { Paint::Red } else { Paint::Dark });
        if material.0 != want {
            material.0 = want;
        }
    }
}

/// Power-ups: spinning, bobbing, blinking out at the end.
fn update_drops(mut commands: Commands, art: Res<Art>, time: Res<Time>, state: Res<MatchState>, mut looks: Query<(Entity, &DropLook, &mut Transform, &mut Visibility)>) {
    let drops = &state.horde.drops;
    let t = time.elapsed_secs();
    let mut have = vec![false; drops.len()];
    for (entity, look, mut transform, mut v) in &mut looks {
        let Some(d) = drops.get(look.0) else {
            commands.entity(entity).despawn();
            continue;
        };
        have[look.0] = true;
        transform.translation = d.at + Vec3::Y * (t * 2.5).sin() * 0.15;
        transform.rotation = Quat::from_rotation_y(t * 2.0);
        let blink = d.ttl > 6.0 || (t * 8.0).sin() > 0.0;
        *v = if blink { Visibility::Inherited } else { Visibility::Hidden };
    }
    for (i, d) in drops.iter().enumerate() {
        if have[i] {
            continue;
        }
        let root = commands.spawn((MatchEntity, DropLook(i), Transform::from_translation(d.at), Visibility::Inherited)).id();
        const PAINTS: [Paint; 7] = [Paint::Glow, Paint::Green, Paint::Red, Paint::Spark, Paint::Wood, Paint::Gold, Paint::Aether];
        const SHAPES: [Shape; 7] = [Shape::Cube, Shape::Cylinder, Shape::Shard, Shape::Cone, Shape::Cube, Shape::Cylinder, Shape::Shard];
        let paint = PAINTS[d.kind as usize % PAINTS.len()];
        let shape = SHAPES[d.kind as usize % SHAPES.len()];
        piece(&mut commands, &art, root, shape, paint, Vec3::ZERO, Vec3::splat(0.45), Quat::from_rotation_z(0.4));
        piece(&mut commands, &art, root, Shape::Cylinder, Paint::Aether, Vec3::new(0.0, -0.4, 0.0), Vec3::new(0.7, 0.02, 0.7), Quat::IDENTITY);
    }
    // A power-up that went away leaves a stale index behind: rebuild when the list shrinks.
    if looks.iter().count() > drops.len() {
        for (entity, ..) in &looks {
            commands.entity(entity).despawn();
        }
    }
}

/// The husk rig: joints we swing every frame.
#[derive(Component)]
struct HuskRig {
    body: Entity,
    torso: Entity,
    head: Entity,
    arms: [Entity; 2],
    legs: [Entity; 2],
    phase: f32,
    last: Vec3,
    speed: f32,
    swing: f32,
    counter: u8,
    seed: f32,
}

fn joint(commands: &mut Commands, parent: Entity, at: Vec3) -> Entity {
    commands.spawn((Transform::from_translation(at), Visibility::Inherited, ChildOf(parent))).id()
}

fn attach_husks(mut commands: Commands, art: Res<Art>, husks: Query<(Entity, &Enemy, &Transform), Added<Enemy>>) {
    for (entity, enemy, transform) in &husks {
        let brute = enemy.kind == kind::BRUTE;
        let crawler = enemy.kind == kind::CRAWLER;
        let volt = enemy.kind == kind::VOLT;
        let (shell, plate, eye) = if brute {
            (Paint::Iron, Paint::Rust, Paint::Spark)
        } else if volt {
            (Paint::Steel, Paint::Teal, Paint::Aether)
        } else {
            (Paint::Rust, Paint::Steel, Paint::Red)
        };
        let scale = if brute { 1.4 } else { 1.0 };
        commands.entity(entity).insert(Visibility::Inherited);
        let body = commands.spawn((Transform::from_scale(Vec3::splat(scale)), Visibility::Inherited, ChildOf(entity))).id();
        let hips_y = if crawler { 0.35 } else { 0.9 };
        let hips = joint(&mut commands, body, Vec3::Y * hips_y);
        piece(&mut commands, &art, hips, Shape::Cube, Paint::Dark, Vec3::ZERO, Vec3::new(0.36, 0.16, 0.22), Quat::IDENTITY);
        let torso = joint(&mut commands, hips, Vec3::Y * 0.06);
        // A gutted chassis: a dented casing, a torn plate, wires spilling out, a dim core.
        piece(&mut commands, &art, torso, Shape::Cube, shell, Vec3::new(0.0, 0.3, 0.0), Vec3::new(0.46, 0.46, 0.3), Quat::from_rotation_z(0.08));
        piece(&mut commands, &art, torso, Shape::Cube, plate, Vec3::new(0.1, 0.34, -0.16), Vec3::new(0.24, 0.26, 0.03), Quat::from_rotation_z(-0.3));
        piece(&mut commands, &art, torso, Shape::Cube, Paint::Copper, Vec3::new(-0.12, 0.1, -0.16), Vec3::new(0.03, 0.22, 0.03), Quat::from_rotation_z(0.5));
        piece(&mut commands, &art, torso, Shape::Cube, Paint::Copper, Vec3::new(-0.05, 0.06, -0.15), Vec3::new(0.025, 0.18, 0.025), Quat::from_rotation_z(-0.4));
        piece(&mut commands, &art, torso, Shape::Shard, if brute { Paint::Spark } else { Paint::Red }, Vec3::new(-0.08, 0.32, -0.16), Vec3::splat(0.07), Quat::IDENTITY);
        piece(&mut commands, &art, torso, Shape::Cube, Paint::Dark, Vec3::new(0.0, 0.3, 0.17), Vec3::new(0.3, 0.32, 0.08), Quat::IDENTITY);
        if volt {
            // An overcharged cell bulging out of the chest, with coils round it: shoot it from afar.
            piece(&mut commands, &art, torso, Shape::Shard, Paint::Aether, Vec3::new(0.0, 0.32, -0.2), Vec3::new(0.2, 0.24, 0.14), Quat::IDENTITY);
            for y in [0.18, 0.46] {
                piece(&mut commands, &art, torso, Shape::Cube, Paint::Copper, Vec3::new(0.0, y, -0.17), Vec3::new(0.36, 0.035, 0.04), Quat::IDENTITY);
            }
            piece(&mut commands, &art, torso, Shape::Cylinder, Paint::Aether, Vec3::new(0.0, 0.62, 0.12), Vec3::new(0.06, 0.3, 0.06), Quat::from_rotation_x(0.3));
        }
        if brute {
            piece(&mut commands, &art, torso, Shape::Cube, Paint::Iron, Vec3::new(0.3, 0.52, 0.0), Vec3::new(0.24, 0.16, 0.34), Quat::from_rotation_z(-0.3));
            piece(&mut commands, &art, torso, Shape::Cube, Paint::Iron, Vec3::new(-0.3, 0.52, 0.0), Vec3::new(0.24, 0.16, 0.34), Quat::from_rotation_z(0.3));
        }
        let head = joint(&mut commands, torso, Vec3::new(0.0, 0.56, -0.03));
        // A cracked television head, the screen dead but for two burning eyes.
        piece(&mut commands, &art, head, Shape::Cube, plate, Vec3::new(0.0, 0.15, 0.0), Vec3::new(0.34, 0.27, 0.25), Quat::from_rotation_z(-0.12));
        piece(&mut commands, &art, head, Shape::Cube, Paint::Screen, Vec3::new(0.0, 0.15, -0.128), Vec3::new(0.27, 0.19, 0.01), Quat::from_rotation_z(-0.12));
        for x in [-0.07, 0.07] {
            piece(&mut commands, &art, head, Shape::Cube, eye, Vec3::new(x, 0.17 - x * 0.12, -0.137), Vec3::new(0.06, 0.035, 0.01), Quat::from_rotation_z(-0.12 + x * 2.0));
        }
        piece(&mut commands, &art, head, Shape::Cube, Paint::Dark, Vec3::new(0.06, 0.08, -0.135), Vec3::new(0.12, 0.012, 0.01), Quat::from_rotation_z(0.4));
        piece(&mut commands, &art, head, Shape::Cube, Paint::Copper, Vec3::new(0.05, 0.32, 0.0), Vec3::new(0.015, 0.14, 0.015), Quat::from_rotation_z(0.6));
        let mut arms = [Entity::PLACEHOLDER; 2];
        for (i, side) in [-1.0f32, 1.0].into_iter().enumerate() {
            let shoulder = joint(&mut commands, torso, Vec3::new(0.29 * side, 0.46, 0.0));
            let long = if brute { 0.8 } else { 0.66 };
            piece(&mut commands, &art, shoulder, Shape::Cube, shell, Vec3::new(0.0, -0.17, 0.0), Vec3::new(0.11, 0.34, 0.11), Quat::IDENTITY);
            piece(&mut commands, &art, shoulder, Shape::Cube, Paint::Dark, Vec3::new(0.0, -long + 0.12, 0.0), Vec3::new(0.09, 0.34, 0.09), Quat::IDENTITY);
            // Claws.
            for k in [-0.03, 0.03] {
                piece(&mut commands, &art, shoulder, Shape::Cone, Paint::Steel, Vec3::new(k, -long - 0.08, 0.0), Vec3::new(0.03, 0.12, 0.03), Quat::from_rotation_x(PI));
            }
            arms[i] = shoulder;
        }
        let mut legs = [Entity::PLACEHOLDER; 2];
        for (i, side) in [-1.0f32, 1.0].into_iter().enumerate() {
            let hip = joint(&mut commands, hips, Vec3::new(0.12 * side, -0.02, 0.0));
            if !crawler {
                piece(&mut commands, &art, hip, Shape::Cube, shell, Vec3::new(0.0, -0.22, 0.0), Vec3::new(0.15, 0.44, 0.16), Quat::IDENTITY);
                piece(&mut commands, &art, hip, Shape::Cube, Paint::Dark, Vec3::new(0.0, -0.66, 0.02), Vec3::new(0.13, 0.44, 0.14), Quat::IDENTITY);
                piece(&mut commands, &art, hip, Shape::Cube, plate, Vec3::new(0.0, -0.88, -0.05), Vec3::new(0.16, 0.07, 0.27), Quat::IDENTITY);
            } else {
                // Stumps trailing wires.
                piece(&mut commands, &art, hip, Shape::Cube, shell, Vec3::new(0.0, -0.08, 0.0), Vec3::new(0.15, 0.16, 0.16), Quat::IDENTITY);
                piece(&mut commands, &art, hip, Shape::Cube, Paint::Copper, Vec3::new(0.0, -0.18, 0.05), Vec3::new(0.02, 0.16, 0.02), Quat::from_rotation_x(0.6));
            }
            legs[i] = hip;
        }
        let seed = (entity.index() % 97) as f32 * 0.37;
        commands.entity(entity).insert(HuskRig { body, torso, head, arms, legs, phase: seed, last: transform.translation, speed: 0.0, swing: 0.0, counter: enemy.attacks, seed });
    }
}

fn animate_husks(time: Res<Time>, mut rigs: Query<(&Transform, &Enemy, &mut HuskRig)>, mut joints: Query<&mut Transform, Without<HuskRig>>) {
    let dt = time.delta_secs().max(1e-4);
    let t = time.elapsed_secs();
    for (transform, enemy, mut rig) in &mut rigs {
        let moved = (transform.translation - rig.last).xz().length() / dt;
        rig.last = transform.translation;
        let moved = if moved > 20.0 { 0.0 } else { moved };
        rig.speed += (moved - rig.speed) * (dt * 8.0).min(1.0);
        let speed = rig.speed;
        let run = ((speed - 2.0) / 2.5).clamp(0.0, 1.0);
        rig.phase += dt * (0.8 + speed * 1.15);
        if enemy.attacks != rig.counter {
            rig.counter = enemy.attacks;
            rig.swing = 1.0;
        }
        rig.swing = (rig.swing - dt * 2.6).max(0.0);
        let p = rig.phase * TAU * 0.5;
        let seed = rig.seed;
        let crawler = enemy.kind == kind::CRAWLER;
        let walking = (speed / 1.2).min(1.0);
        let set = |joints: &mut Query<&mut Transform, Without<HuskRig>>, e: Entity, rot: Quat| {
            if let Ok(mut j) = joints.get_mut(e) {
                j.rotation = rot;
            }
        };
        // A shambler drags one leg and lists; a runner leans in and pumps its arms.
        let lean = if crawler { 1.35 } else { 0.28 + 0.35 * run };
        let sway = (p * 0.5).sin() * 0.14 * (1.0 - run) + (t * 0.7 + seed).sin() * 0.05;
        set(&mut joints, rig.torso, Quat::from_rotation_x(-lean) * Quat::from_rotation_z(sway));
        set(&mut joints, rig.head, Quat::from_rotation_x(if crawler { 1.1 } else { 0.25 }) * Quat::from_rotation_z((t * 1.9 + seed).sin() * 0.25 + 0.2) * Quat::from_rotation_y((t * 0.6 + seed * 3.0).sin() * 0.3));
        if let Ok(mut body) = joints.get_mut(rig.body) {
            let scale = body.scale;
            body.translation.y = (p * 2.0).sin().abs() * 0.05 * walking;
            body.scale = scale;
        }
        for (i, side) in [-1.0f32, 1.0].into_iter().enumerate() {
            let phase = p + i as f32 * PI;
            let drag = if i == 0 { 0.45 } else { 1.0 };
            let stride = (0.25 + 0.45 * run) * walking * if run > 0.5 { 1.0 } else { drag };
            set(&mut joints, rig.legs[i], Quat::from_rotation_x(phase.sin() * stride));
            // Arms: reaching out, clawing on a swing; pumping on a run; paddling on a crawl.
            let reach = if crawler {
                -1.6 + phase.sin() * 0.7
            } else {
                let forward = -1.35 + (t * 2.3 + seed + i as f32).sin() * 0.12;
                let pump = -0.4 - phase.cos() * 0.8;
                forward * (1.0 - run) + pump * run
            };
            let claw = rig.swing * if i == 0 { 1.4 } else { 1.0 };
            let up = (rig.swing * PI).sin();
            set(&mut joints, rig.arms[i], Quat::from_rotation_x(reach - up * 1.2 + claw * 0.9) * Quat::from_rotation_z(side * (0.12 + 0.1 * (1.0 - run))));
        }
    }
}

/// The local player's Use: once on the press, then a steady "still held".
#[allow(clippy::too_many_arguments)]
fn send_use(
    time: Res<Time<Real>>,
    controls: Res<Controls>,
    settings: Res<Settings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    gamepads: Query<&Gamepad>,
    screen: Res<Screen>,
    me: Query<&Player, With<LocalPlayer>>,
    mut orders: EventWriter<CommandEvent>,
    mut repeat: Local<f32>,
) {
    let Ok(me) = me.single() else { return };
    if *screen != Screen::None {
        return;
    }
    let held = settings.bindings.get(Action::Interact).pressed(&keys, &mouse) || gamepads.iter().any(|g| g.pressed(GamepadButton::West));
    if controls.interact {
        orders.write(CommandEvent { player: me.id, command: PlayerCommand::Mode(act::USE) });
        *repeat = 0.12;
    }
    if held {
        *repeat -= time.delta_secs();
        if *repeat <= 0.0 {
            *repeat = 0.12;
            orders.write(CommandEvent { player: me.id, command: PlayerCommand::Mode(act::HOLD) });
        }
    }
}

#[derive(Component)]
struct RoundText;
#[derive(Component)]
struct RoundLabel;
#[derive(Component)]
struct PointsText;
#[derive(Component)]
struct PerkRow;
#[derive(Component)]
struct UsePrompt;
#[derive(Component)]
struct Status;
#[derive(Component)]
struct BarBack;
#[derive(Component)]
struct BarFill;
#[derive(Component)]
struct PowerUps;
#[derive(Component)]
struct Popup(f32);
#[derive(Component)]
struct PopupAnchor;

fn spawn_hud(mut commands: Commands) {
    let at = |node: Node| Node { position_type: PositionType::Absolute, ..node };
    commands.spawn((MatchEntity, at(Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }), GlobalZIndex(-38))).with_children(|hud| {
        // Top left: the round, big and red, and the perk badges under it.
        hud.spawn(at(Node { left: Val::Px(22.0), top: Val::Px(12.0), flex_direction: FlexDirection::Column, row_gap: Val::Px(2.0), ..default() })).with_children(|corner| {
            corner.spawn((RoundLabel, Text::new("ROUND"), TextFont { font_size: 14.0, ..default() }, TextColor(Color::srgb(0.75, 0.2, 0.18))));
            corner.spawn((RoundText, Text::new(""), TextFont { font_size: 72.0, ..default() }, TextColor(Color::srgb(0.85, 0.1, 0.08)), TextShadow::default()));
            corner.spawn((PerkRow, Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(5.0), ..default() }));
        });
        // Right, above the weapon panel: everyone's points.
        hud.spawn(at(Node { right: Val::Px(22.0), bottom: Val::Px(150.0), flex_direction: FlexDirection::Column, align_items: AlignItems::End, ..default() })).with_children(|col| {
            col.spawn((PopupAnchor, Node { height: Val::Px(0.0), ..default() }));
            col.spawn((PointsText, Text::new(""), TextFont { font_size: 18.0, ..default() }, TextColor(ui::TEXT), TextLayout::new_with_justify(JustifyText::Right), TextShadow::default()));
        });
        // Lower middle: what Use does here; status (down, reviving, next round); the bar.
        hud.spawn(at(Node { width: Val::Percent(100.0), top: Val::Percent(64.0), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, row_gap: Val::Px(6.0), ..default() })).with_children(|mid| {
            mid.spawn((UsePrompt, Text::new(""), TextFont { font_size: 19.0, ..default() }, TextColor(ui::ACCENT), TextShadow::default()));
            mid.spawn((Status, Text::new(""), TextFont { font_size: 22.0, ..default() }, TextColor(ui::WARN), TextLayout::new_with_justify(JustifyText::Center), TextShadow::default()));
            mid.spawn((BarBack, Node { width: Val::Px(260.0), height: Val::Px(10.0), ..default() }, BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)), Visibility::Hidden)).with_children(|bar| {
                bar.spawn((BarFill, Node { width: Val::Percent(0.0), height: Val::Percent(100.0), ..default() }, BackgroundColor(ui::GOOD)));
            });
        });
        // Bottom middle: power-ups running.
        hud.spawn(at(Node { width: Val::Percent(100.0), bottom: Val::Px(26.0), justify_content: JustifyContent::Center, ..default() })).with_children(|bottom| {
            bottom.spawn((PowerUps, Text::new(""), TextFont { font_size: 17.0, ..default() }, TextColor(Color::srgb(0.6, 1.0, 0.55)), TextShadow::default()));
        });
    });
}

fn paint_color(paint: Paint) -> Color {
    let [r, g, b] = paint.rgb();
    Color::srgb(r, g, b)
}

#[allow(clippy::too_many_arguments)]
fn update_hud(
    mut commands: Commands,
    map: Res<Map>,
    state: Res<MatchState>,
    content: Res<Content>,
    settings: Res<Settings>,
    controls: Res<Controls>,
    camera: Query<&Transform, With<WorldCamera>>,
    me: Query<(&Player, &Transform, &Arsenal, &Health), With<LocalPlayer>>,
    others: Query<(&Player, &Transform)>,
    mut prompt: ResMut<Prompt>,
    mut texts: ParamSet<(
        Query<&mut Text, With<RoundText>>,
        Query<(&mut Text, &mut TextColor), With<UsePrompt>>,
        Query<(&mut Text, &mut TextColor), With<Status>>,
        Query<&mut Text, With<PointsText>>,
        Query<&mut Text, With<PowerUps>>,
    )>,
    mut bar: Query<&mut Visibility, With<BarBack>>,
    mut fill: Query<(&mut Node, &mut BackgroundColor), With<BarFill>>,
    perk_row: Query<(Entity, Option<&Children>), With<PerkRow>>,
    anchor: Query<Entity, With<PopupAnchor>>,
    mut shown: Local<(u8, u32)>,
) {
    let Some(layout) = map.horde.as_deref() else { return };
    let h = &state.horde;
    let Ok((player, transform, arsenal, health)) = me.single() else { return };
    let id = player.id;
    let mine = h.survivor(id);
    let name_of = |pid: u8| state.roster.iter().find(|r| r.id == pid).map_or(format!("PLAYER {}", pid + 1), |r| r.name.clone());

    if let Ok(mut text) = texts.p0().single_mut() {
        let round = if h.round == 0 { String::new() } else { h.round.to_string() };
        if text.0 != round {
            text.0 = round;
        }
    }

    // Points: you first and bigger, then the squad.
    let mut lines = Vec::new();
    if let Some(s) = mine {
        lines.push(format!("{}   {}", name_of(id), s.points));
    }
    for s in h.survivors.iter().filter(|s| s.id != id) {
        if state.roster.iter().any(|r| r.id == s.id) {
            let tag = if s.down { "  [DOWN]" } else if s.out { "  [OUT]" } else { "" };
            lines.push(format!("{}{}   {}", name_of(s.id), tag, s.points));
        }
    }
    if let Ok(mut text) = texts.p3().single_mut() {
        let joined = lines.join("\n");
        if text.0 != joined {
            text.0 = joined;
        }
    }
    // +points popups.
    if let Some(s) = mine {
        if shown.0 != id || s.points < shown.1 {
            *shown = (id, s.points);
        } else if s.points > shown.1 {
            let gained = s.points - shown.1;
            *shown = (id, s.points);
            if let Ok(anchor) = anchor.single() {
                let color = if gained >= 100 { Color::srgb(1.0, 0.85, 0.3) } else { Color::srgb(0.95, 0.95, 0.6) };
                commands.spawn((Popup(0.0), Text::new(format!("+{gained}")), TextFont { font_size: if gained >= 50 { 22.0 } else { 17.0 }, ..default() }, TextColor(color), TextShadow::default(), Node { position_type: PositionType::Absolute, right: Val::Px(-10.0), bottom: Val::Px(0.0), ..default() }, ChildOf(anchor)));
            }
        }
    }
    // Perk badges.
    if let (Some(s), Ok((row, children))) = (mine, perk_row.single()) {
        let count = children.map_or(0, |c| c.len());
        if count != s.perks.count_ones() as usize {
            commands.entity(row).despawn_related::<Children>();
            for p in 0..perk::COUNT as u8 {
                if s.has(p) {
                    let color = paint_color(perk::COLORS[p as usize]);
                    commands.spawn((Node { padding: UiRect::axes(Val::Px(5.0), Val::Px(2.0)), border: UiRect::all(Val::Px(2.0)), ..default() }, BackgroundColor(color.with_alpha(0.35)), BorderColor(color), ChildOf(row))).with_children(|badge| {
                        badge.spawn((Text::new(perk::SHORT[p as usize]), TextFont { font_size: 14.0, ..default() }, TextColor(Color::WHITE)));
                    });
                }
            }
        }
    }

    // What Use does here.
    let look = camera.single().map(|c| c.forward().as_vec3()).unwrap_or(Vec3::NEG_Z);
    let downed: Vec<(u8, Vec3)> = others.iter().filter(|(p, _)| h.is_down(p.id)).map(|(p, t)| (p.id, t.translation)).collect();
    let what = if health.alive() && !state.over() { zombies::interaction(layout, h, id, transform.translation, look, &downed) } else { None };
    prompt.0 = what;
    let key = if controls.gamepad { "X".to_string() } else { settings.bindings.label(Action::Interact) };
    if let Ok((mut text, mut color)) = texts.p1().single_mut() {
        let (line, tint) = match what {
            Some(w) => {
                let (label, price) = zombies::describe(layout, h, &content, id, arsenal, w, name_of);
                let afford = price.is_none_or(|p| mine.is_some_and(|s| s.points >= p));
                let verb = if w.held() { format!("HOLD [{key}]") } else { format!("[{key}]") };
                (match price {
                    Some(p) => format!("{verb} {label}  -  {p}"),
                    None => format!("{verb} {label}"),
                }, if afford { ui::ACCENT } else { ui::WARN })
            }
            None => (String::new(), ui::ACCENT),
        };
        if text.0 != line {
            text.0 = line;
        }
        if color.0 != tint {
            color.0 = tint;
        }
    }

    // Status: down and bleeding, getting someone up, the next round coming.
    let mut bar_value: Option<(f32, Color)> = None;
    let status = if let Some(s) = mine.filter(|s| s.down) {
        if s.revive > 0.0 {
            bar_value = Some((s.revive, ui::GOOD));
            format!("{} IS GETTING YOU UP", name_of(s.reviver))
        } else if s.self_revive > 0.0 {
            bar_value = Some((1.0 - s.self_revive / 4.0, ui::ACCENT));
            "PATCHING YOURSELF UP".to_string()
        } else {
            bar_value = Some((s.bleed / zombies::BLEED, ui::WARN));
            format!("YOU ARE DOWN - BLEEDING OUT  {:.0}", s.bleed.ceil())
        }
    } else if let Some(patient) = h.survivors.iter().find(|s| s.down && s.reviver == id) {
        bar_value = Some((patient.revive, ui::GOOD));
        format!("REVIVING {}", name_of(patient.id))
    } else if what == Some(Interact::Power) && h.charge > 0.0 {
        bar_value = Some((h.charge, ui::ACCENT));
        "THE GENERATOR SPUTTERS".to_string()
    } else if mine.is_some_and(|s| s.out) || !health.alive() {
        "BLED OUT - BACK NEXT ROUND".to_string()
    } else if h.rest > 0.0 && !state.over() {
        if h.round == 0 { format!("THEY ARE COMING  {:.0}", h.rest.ceil()) } else { format!("ROUND {} IN {:.0}", h.round + 1, h.rest.ceil()) }
    } else {
        let down: Vec<String> = h.survivors.iter().filter(|s| s.down && s.id != id).map(|s| name_of(s.id)).collect();
        if down.is_empty() { String::new() } else { format!("{} IS DOWN - HOLD [{key}] ON THEM", down.join(", ")) }
    };
    if let Ok((mut text, mut color)) = texts.p2().single_mut() {
        if text.0 != status {
            text.0 = status;
        }
        let tint = if mine.is_some_and(|s| s.down) { ui::WARN } else { ui::TEXT };
        if color.0 != tint {
            color.0 = tint;
        }
    }
    if let Ok(mut v) = bar.single_mut() {
        let want = if bar_value.is_some() { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
    if let (Some((value, tint)), Ok((mut node, mut color))) = (bar_value, fill.single_mut()) {
        node.width = Val::Percent((value.clamp(0.0, 1.0) * 100.0).round());
        color.0 = tint;
    }
    if let Ok(mut text) = texts.p4().single_mut() {
        let mut parts = Vec::new();
        if h.double > 0.0 {
            parts.push(format!("{}  {:.0}", drop::NAMES[drop::DOUBLE as usize], h.double.ceil()));
        }
        if h.insta > 0.0 {
            parts.push(format!("{}  {:.0}", drop::NAMES[drop::INSTA as usize], h.insta.ceil()));
        }
        if h.sale > 0.0 {
            parts.push(format!("{}  {:.0}", drop::NAMES[drop::CLEARANCE as usize], h.sale.ceil()));
        }
        for i in 0..layout.traps.len() {
            let (running, left) = h.trap(i);
            if running {
                parts.push(format!("ARC GRATE  {:.0}", left.ceil()));
            }
        }
        if h.relays != 0 && (h.relays.count_ones() as usize) < layout.relays.len() {
            parts.push(format!("RELAYS {}/{}", h.relays.count_ones(), layout.relays.len()));
        }
        let line = parts.join("      ");
        if text.0 != line {
            text.0 = line;
        }
    }
}

fn float_popups(mut commands: Commands, time: Res<Time<Real>>, mut popups: Query<(Entity, &mut Popup, &mut Node, &mut TextColor)>) {
    let dt = time.delta_secs();
    for (entity, mut popup, mut node, mut color) in &mut popups {
        popup.0 += dt;
        node.bottom = Val::Px(popup.0 * 60.0);
        color.0 = color.0.with_alpha((1.2 - popup.0).clamp(0.0, 1.0));
        if popup.0 > 1.2 {
            commands.entity(entity).despawn();
        }
    }
}

/// The game-over summary for the horde: rounds survived, and everyone's kills and points.
pub fn game_over(state: &MatchState) -> (String, String) {
    let h = &state.horde;
    let rounds = h.round.saturating_sub(1);
    let headline = format!("OVERRUN - {} ROUND{} SURVIVED", rounds, if rounds == 1 { "" } else { "S" });
    let mut lines = vec![format!("{} HUSKS SCRAPPED", h.kills)];
    for s in &h.survivors {
        let name = state.roster.iter().find(|r| r.id == s.id).map_or(format!("PLAYER {}", s.id + 1), |r| r.name.clone());
        lines.push(format!("{name}   {} KILLS   {} HEADSHOTS   {} POINTS   {} DOWNS   {} REVIVES", s.kills, s.headshots, s.points, s.downs, s.revives));
    }
    (headline, lines.join("\n"))
}
