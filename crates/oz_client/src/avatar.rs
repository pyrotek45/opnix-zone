//! Robot operators with television heads.
//!
//! Every player is a jointed robot: hips, two-part legs, torso, an aiming
//! rig carrying arms and the gun, and a boxy CRT head whose screen shows a
//! face. The rig is animated procedurally from a [`Pose`] - run cycles,
//! crouching, going prone, looking up and down, reloading, sprinting and
//! falling over - and the face changes with what the robot is doing.
//!
//! The same rig is used in the world (other players and bots) and in the
//! showroom (menus, lobby, model lineup).

use crate::{
    ClientSet,
    art::{Art, Baked},
};
use bevy::{pbr::NotShadowCaster, platform::collections::HashMap, prelude::*};
use oz_core::arsenal::{SLOT_COUNT, assemble};
use oz_sim::prelude::*;
use std::f32::consts::{FRAC_PI_2, PI};

pub const HIP: f32 = 0.92;
const THIGH: f32 = 0.42;
const NECK: f32 = 0.56;
const SHOULDER: f32 = 0.44;
/// Where the gun's grip sits relative to the shoulder line.
const GRIP: Vec3 = Vec3::new(0.1, -0.12, -0.36);

/// Facial expressions, in the order they are baked.
pub mod face {
    pub const SMILE: usize = 0;
    pub const ANGRY: usize = 1;
    pub const HURT: usize = 2;
    pub const DEAD: usize = 3;
    pub const BLINK: usize = 4;
    pub const COUNT: usize = 5;
}

/// Team colours for the robot's casing and plates; 2 = neutral (menus).
pub fn team_paint(team: u8) -> Paint {
    [Paint::Teal, Paint::Coral, Paint::Bone][(team as usize).min(2)]
}

/// Everything a robot looks like, for one team colour.
struct RobotMeshes {
    hips: Baked,
    thigh: Baked,
    shin: Baked,
    torso: Baked,
    head: Baked,
    arms: Baked,
    faces: Vec<Baked>,
}

#[derive(Resource)]
pub struct RobotParts {
    teams: Vec<RobotMeshes>,
}

/// What a robot should be doing. Filled from gameplay for players, or set directly in the showroom.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Pose {
    /// Signed ground speed along the facing direction, m/s.
    pub speed: f32,
    pub stance: u8,
    pub pitch: f32,
    pub aiming: bool,
    pub reloading: bool,
    pub sprinting: bool,
    pub dead: bool,
    /// Force a face (showroom); otherwise the face follows events.
    pub face: Option<usize>,
}

/// Joint handles and animation state for one robot.
#[derive(Component)]
pub struct Rig {
    body: Entity,
    hips: Entity,
    thighs: [Entity; 2],
    shins: [Entity; 2],
    torso: Entity,
    head: Entity,
    aim: Entity,
    gun: Entity,
    faces: [Entity; face::COUNT],
    phase: f32,
    dead_for: f32,
    blink: f32,
    pub angry: f32,
    pub hurt: f32,
    last_hp: f32,
    last_pos: Vec3,
    /// The weapon (index, parts) currently in its hands.
    held: Option<(u8, [u8; SLOT_COUNT])>,
}

/// Marks a robot that belongs to a player in the world.
#[derive(Component)]
pub struct PlayerRobot;

pub struct AvatarPlugin;

impl Plugin for AvatarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GunCache>().add_systems(Startup, bake_robots).add_systems(
            Update,
            ((attach_to_players, pose_players, react_to_shots, hold_weapons).run_if(in_state(MatchPhase::Active)), animate_rigs).chain().in_set(ClientSet::Present),
        );
    }
}

fn eye(paint: Paint, x: f32, y: f32) -> BlockModel {
    BlockModel::new().cube(paint, [x, y, -0.15], [0.036, 0.05, 0.008])
}

/// The face layer drawn on the TV screen, in neck-pivot space.
fn face_model(expression: usize) -> BlockModel {
    let z = -0.15;
    let pixel = |m: BlockModel, paint: Paint, x: f32, y: f32, w: f32, h: f32| m.cube(paint, [x, y, z], [w, h, 0.008]);
    let smile_mouth = |m: BlockModel, paint: Paint| {
        let m = pixel(m, paint, -0.055, 0.135, 0.024, 0.024);
        let m = pixel(m, paint, -0.028, 0.118, 0.026, 0.022);
        let m = pixel(m, paint, 0.0, 0.112, 0.03, 0.022);
        let m = pixel(m, paint, 0.028, 0.118, 0.026, 0.022);
        pixel(m, paint, 0.055, 0.135, 0.024, 0.024)
    };
    match expression {
        face::ANGRY => {
            let paint = Paint::Red;
            let m = BlockModel::new()
                .segment(paint, [-0.1, 0.235, z], [-0.03, 0.212, z], 0.02)
                .segment(paint, [0.1, 0.235, z], [0.03, 0.212, z], 0.02)
                .merged(&eye(paint, -0.06, 0.188), [0.0; 3])
                .merged(&eye(paint, 0.06, 0.188), [0.0; 3]);
            pixel(m, paint, 0.0, 0.122, 0.11, 0.02)
        }
        face::HURT => {
            let paint = Paint::Glow;
            let m = BlockModel::new()
                .segment(paint, [-0.1, 0.225, z], [-0.04, 0.2, z], 0.018)
                .segment(paint, [-0.04, 0.2, z], [-0.1, 0.175, z], 0.018)
                .segment(paint, [0.1, 0.225, z], [0.04, 0.2, z], 0.018)
                .segment(paint, [0.04, 0.2, z], [0.1, 0.175, z], 0.018);
            let m = pixel(m, paint, -0.02, 0.125, 0.012, 0.04);
            let m = pixel(m, paint, 0.02, 0.125, 0.012, 0.04);
            let m = pixel(m, paint, 0.0, 0.145, 0.04, 0.012);
            pixel(m, paint, 0.0, 0.105, 0.04, 0.012)
        }
        face::DEAD => {
            let paint = Paint::Red;
            let mut m = BlockModel::new();
            for x in [-0.065, 0.065] {
                m = m
                    .segment(paint, [x - 0.03, 0.23, z], [x + 0.03, 0.17, z], 0.016)
                    .segment(paint, [x + 0.03, 0.23, z], [x - 0.03, 0.17, z], 0.016);
            }
            pixel(m, paint, 0.0, 0.12, 0.09, 0.016)
        }
        face::BLINK => {
            let paint = Paint::Aether;
            let m = pixel(BlockModel::new(), paint, -0.06, 0.19, 0.05, 0.012);
            let m = pixel(m, paint, 0.06, 0.19, 0.05, 0.012);
            smile_mouth(m, paint)
        }
        _ => {
            let paint = Paint::Aether;
            let m = BlockModel::new().merged(&eye(paint, -0.06, 0.2), [0.0; 3]).merged(&eye(paint, 0.06, 0.2), [0.0; 3]);
            smile_mouth(m, paint)
        }
    }
}

fn head_model(team: Paint) -> BlockModel {
    BlockModel::new()
        .cylinder(Paint::Steel, [0.0, 0.02, 0.0], [0.09, 0.06, 0.09])
        // The set: casing, bezel, screen, and a deeper back.
        .cube(team, [0.0, 0.17, 0.0], [0.38, 0.29, 0.26])
        .cube(Paint::Dark, [0.0, 0.17, 0.11], [0.3, 0.22, 0.12])
        .cube(Paint::Dark, [0.0, 0.17, -0.131], [0.33, 0.25, 0.012])
        .cube(Paint::Screen, [0.0, 0.17, -0.138], [0.29, 0.21, 0.006])
        // Dials and a speaker grille on the right of the screen.
        .cylinder(Paint::Metal, [0.165, 0.21, -0.12], [0.03, 0.02, 0.03])
        .rotated([90.0, 0.0, 0.0])
        .cylinder(Paint::Gold, [0.165, 0.15, -0.12], [0.025, 0.02, 0.025])
        .rotated([90.0, 0.0, 0.0])
        // Rabbit-ear antennae.
        .segment(Paint::Metal, [-0.05, 0.31, 0.0], [-0.16, 0.5, 0.02], 0.014)
        .segment(Paint::Metal, [0.05, 0.31, 0.0], [0.15, 0.53, 0.02], 0.014)
        .shard(Paint::Red, [-0.16, 0.5, 0.02], [0.025, 0.025, 0.025])
        .shard(Paint::Glow, [0.15, 0.53, 0.02], [0.025, 0.025, 0.025])
        .cube(Paint::Dark, [0.0, 0.315, 0.0], [0.12, 0.02, 0.08])
}

fn torso_model(team: Paint) -> BlockModel {
    BlockModel::new()
        .cube(Paint::Dark, [0.0, 0.04, 0.0], [0.34, 0.14, 0.22])
        .cube(Paint::Steel, [0.0, 0.3, 0.0], [0.46, 0.44, 0.28])
        .cube(team, [0.0, 0.34, -0.145], [0.34, 0.22, 0.02])
        .shard(Paint::Glow, [0.0, 0.34, -0.16], [0.045, 0.045, 0.02])
        .cube(team, [0.29, 0.48, 0.0], [0.15, 0.12, 0.26])
        .cube(team, [-0.29, 0.48, 0.0], [0.15, 0.12, 0.26])
        .cube(Paint::Dark, [0.0, 0.3, 0.19], [0.32, 0.36, 0.12])
        .cube(Paint::Aether, [0.0, 0.36, 0.252], [0.18, 0.03, 0.006])
        .cube(Paint::Aether, [0.0, 0.3, 0.252], [0.18, 0.03, 0.006])
}

fn arms_model(team: Paint) -> BlockModel {
    let (r_sh, r_el, r_hand) = ([0.27, 0.02, 0.0], [0.25, -0.2, -0.1], [GRIP.x, GRIP.y, GRIP.z + 0.03]);
    let (l_sh, l_el, l_hand) = ([-0.27, 0.02, 0.0], [-0.2, -0.17, -0.3], [0.06, -0.08, -0.66]);
    BlockModel::new()
        .segment(Paint::Steel, r_sh, r_el, 0.1)
        .segment(Paint::Dark, r_el, r_hand, 0.085)
        .shard(team, r_el, [0.07, 0.07, 0.07])
        .cube(Paint::Dark, r_hand, [0.09, 0.09, 0.09])
        .segment(Paint::Steel, l_sh, l_el, 0.1)
        .segment(Paint::Dark, l_el, l_hand, 0.085)
        .shard(team, l_el, [0.07, 0.07, 0.07])
        .cube(Paint::Dark, l_hand, [0.09, 0.09, 0.09])
}

fn thigh_model(team: Paint) -> BlockModel {
    BlockModel::new().cube(Paint::Steel, [0.0, -THIGH * 0.5, 0.0], [0.16, THIGH, 0.18]).cube(team, [0.0, -THIGH + 0.02, -0.095], [0.13, 0.1, 0.03])
}

fn shin_model() -> BlockModel {
    BlockModel::new().cube(Paint::Dark, [0.0, -0.2, 0.0], [0.14, 0.4, 0.16]).cube(Paint::Steel, [0.0, -0.44, -0.05], [0.17, 0.08, 0.29])
}

fn bake_robots(mut commands: Commands, art: Res<Art>, mut meshes: ResMut<Assets<Mesh>>) {
    let teams = (0..3u8)
        .map(|team| {
            let paint = team_paint(team);
            RobotMeshes {
                hips: art.bake(&BlockModel::new().cube(Paint::Dark, [0.0, 0.0, 0.0], [0.36, 0.16, 0.22]), &mut meshes),
                thigh: art.bake(&thigh_model(paint), &mut meshes),
                shin: art.bake(&shin_model(), &mut meshes),
                torso: art.bake(&torso_model(paint), &mut meshes),
                head: art.bake(&head_model(paint), &mut meshes),
                arms: art.bake(&arms_model(paint), &mut meshes),
                faces: (0..face::COUNT).map(|f| art.bake(&face_model(f), &mut meshes)).collect(),
            }
        })
        .collect();
    commands.insert_resource(RobotParts { teams });
}

fn meshes_of(art: &Art, baked: &Baked) -> Vec<(Handle<Mesh>, Handle<StandardMaterial>, bool)> {
    let mut out = Vec::new();
    if let Some(mesh) = &baked.lit {
        out.push((mesh.clone(), art.lit.clone(), false));
    }
    if let Some(mesh) = &baked.unlit {
        out.push((mesh.clone(), art.unlit.clone(), true));
    }
    out
}

fn joint(commands: &mut Commands, art: &Art, parent: Entity, at: Vec3, baked: Option<&Baked>) -> Entity {
    let entity = commands.spawn((Transform::from_translation(at), Visibility::Inherited, ChildOf(parent))).id();
    if let Some(baked) = baked {
        for (mesh, material, unlit) in meshes_of(art, baked) {
            let mut part = commands.spawn((Mesh3d(mesh), MeshMaterial3d(material), ChildOf(entity)));
            if unlit {
                part.insert(NotShadowCaster);
            }
        }
    }
    entity
}

/// Builds a robot under `root` and returns its rig. `gun` is placed in its hands.
pub fn spawn_robot(commands: &mut Commands, art: &Art, parts: &RobotParts, root: Entity, team: u8, gun: Option<&Baked>) -> Rig {
    let m = &parts.teams[(team as usize).min(2)];
    commands.entity(root).insert(Visibility::Inherited);
    let body = joint(commands, art, root, Vec3::ZERO, None);
    let hips = joint(commands, art, body, Vec3::Y * HIP, Some(&m.hips));
    let mut thighs = [Entity::PLACEHOLDER; 2];
    let mut shins = [Entity::PLACEHOLDER; 2];
    for (i, side) in [-1.0, 1.0].into_iter().enumerate() {
        thighs[i] = joint(commands, art, hips, Vec3::new(0.12 * side, -0.02, 0.0), Some(&m.thigh));
        shins[i] = joint(commands, art, thighs[i], Vec3::new(0.0, -THIGH, 0.0), Some(&m.shin));
    }
    let torso = joint(commands, art, hips, Vec3::Y * 0.06, Some(&m.torso));
    let head = joint(commands, art, torso, Vec3::Y * NECK, Some(&m.head));
    let faces: [Entity; face::COUNT] = std::array::from_fn(|f| {
        let entity = joint(commands, art, head, Vec3::ZERO, Some(&m.faces[f]));
        commands.entity(entity).insert(if f == face::SMILE { Visibility::Inherited } else { Visibility::Hidden });
        entity
    });
    let aim = joint(commands, art, torso, Vec3::Y * SHOULDER, Some(&m.arms));
    let gun_root = joint(commands, art, aim, GRIP, gun);
    Rig {
        body, hips, thighs, shins, torso, head, aim, gun: gun_root, faces, phase: 0.0, dead_for: 0.0, blink: 2.0, angry: 0.0, hurt: 0.0,
        last_hp: 0.0, last_pos: Vec3::NAN, held: None,
    }
}

/// Third-person gun meshes, baked once per (weapon, parts) combination.
#[derive(Resource, Default)]
pub struct GunCache(HashMap<(u8, [u8; SLOT_COUNT]), Baked>);

impl GunCache {
    pub fn get(&mut self, art: &Art, meshes: &mut Assets<Mesh>, content: &Content, weapon: u8, parts: [u8; SLOT_COUNT]) -> Option<Baked> {
        let def = content.weapons.get(weapon as usize)?;
        Some(self.0.entry((weapon, parts)).or_insert_with(|| art.bake(&assemble(def, &parts), meshes)).clone())
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

fn attach_to_players(
    mut commands: Commands,
    art: Res<Art>,
    parts: Res<RobotParts>,
    mut cache: ResMut<GunCache>,
    content: Res<Content>,
    added: Query<(Entity, &Player, &Transform), (Added<Player>, Without<LocalPlayer>)>,
) {
    if content.is_added() {
        cache.clear();
    }
    for (entity, player, transform) in &added {
        let mut rig = spawn_robot(&mut commands, &art, &parts, entity, player.team, None);
        rig.last_pos = transform.translation;
        commands.entity(entity).insert((rig, Pose::default(), PlayerRobot));
    }
}

/// Puts the right weapon in each robot's hands.
fn hold_weapons(
    mut commands: Commands,
    art: Res<Art>,
    content: Res<Content>,
    mut cache: ResMut<GunCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut robots: Query<(&mut Rig, &PlayerInput, &Player, Option<&Arsenal>), With<PlayerRobot>>,
    state: Res<MatchState>,
) {
    for (mut rig, input, player, arsenal) in &mut robots {
        // The host knows real attachments; clients learn them from the roster.
        let roster = state.roster.iter().find(|r| r.id == player.id && r.weapon == input.weapon).map(|r| r.parts);
        let parts = arsenal
            .and_then(|a| a.weapons.get(input.weapon as usize))
            .map(|w| w.equipped)
            .or(roster)
            .or_else(|| content.weapons.get(input.weapon as usize).map(|d| d.defaults))
            .unwrap_or([0; SLOT_COUNT]);
        let wanted = Some((input.weapon, parts));
        if rig.held == wanted {
            continue;
        }
        rig.held = wanted;
        commands.entity(rig.gun).despawn_related::<Children>();
        if let Some(baked) = cache.get(&art, &mut meshes, &content, input.weapon, parts) {
            for (mesh, material, _) in meshes_of(&art, &baked) {
                commands.spawn((Mesh3d(mesh), MeshMaterial3d(material), ChildOf(rig.gun)));
            }
        }
    }
}

fn pose_players(time: Res<Time>, mut robots: Query<(&mut Rig, &mut Pose, &Transform, &Player, &PlayerInput, &Health), With<PlayerRobot>>) {
    let dt = time.delta_secs().max(1e-4);
    for (mut rig, mut pose, transform, player, input, health) in &mut robots {
        let moved = if rig.last_pos.is_finite() { transform.translation - rig.last_pos } else { Vec3::ZERO };
        rig.last_pos = transform.translation;
        let forward = transform.rotation * Vec3::NEG_Z;
        let speed = moved.xz().length() / dt;
        let along = if moved.xz().dot(forward.xz()) < -0.01 { -1.0 } else { 1.0 };
        // Smooth jittery network movement into a steady gait.
        pose.speed += (speed.min(9.0) * along - pose.speed) * (1.0 - (-dt * 10.0).exp());
        pose.stance = input.stance.max(player.flags & flags::STANCE);
        pose.pitch = input.pitch;
        pose.aiming = player.flags & flags::AIMING != 0;
        pose.reloading = player.flags & flags::RELOADING != 0;
        pose.sprinting = player.flags & flags::SPRINTING != 0;
        pose.dead = !health.alive();
        if health.hp < rig.last_hp - 0.5 {
            rig.hurt = 0.6;
        }
        rig.last_hp = health.hp;
    }
}

/// Robots scowl while shooting.
fn react_to_shots(mut events: EventReader<FxEvent>, mut robots: Query<(&mut Rig, &Player)>) {
    for event in events.read() {
        if let FxEvent::Shot { by, .. } = event {
            if let Some((mut rig, _)) = robots.iter_mut().find(|(_, player)| player.id == *by) {
                rig.angry = 1.2;
            }
        }
    }
}

fn set(transforms: &mut Query<&mut Transform, Without<Rig>>, entity: Entity, translation: Vec3, rotation: Quat, blend: f32) {
    if let Ok(mut t) = transforms.get_mut(entity) {
        t.translation = t.translation.lerp(translation, blend);
        t.rotation = t.rotation.slerp(rotation, blend);
    }
}

/// Drives every joint from the pose.
pub fn animate_rigs(time: Res<Time>, mut rigs: Query<(&mut Rig, &Pose)>, mut transforms: Query<&mut Transform, Without<Rig>>, mut visibility: Query<&mut Visibility, Without<Rig>>) {
    let dt = time.delta_secs().min(0.05);
    let blend = 1.0 - (-dt * 16.0).exp();
    let rot = |x: f32| Quat::from_rotation_x(x);
    for (mut rig, pose) in &mut rigs {
        // Gait: one full stride per ~1.4 m, running backwards when retreating.
        let k = (pose.speed.abs() / WALK).min(1.6);
        rig.phase += pose.speed * dt * 4.4;
        let p = rig.phase;
        rig.dead_for = if pose.dead { rig.dead_for + dt } else { 0.0 };
        rig.angry = (rig.angry - dt).max(0.0);
        rig.hurt = (rig.hurt - dt).max(0.0);
        rig.blink -= dt;
        if rig.blink < -0.12 {
            rig.blink = 2.0 + (p * 7.3).sin().abs() * 2.5;
        }

        // Whole-body placement: prone lies forward, dead falls backward.
        let (body_at, body_rot) = if pose.dead {
            let fall = (rig.dead_for / 0.35).min(1.0);
            (Vec3::new(0.0, 0.18 * fall, 0.25 * fall), rot(FRAC_PI_2 * fall))
        } else if pose.stance == stance::PRONE {
            (Vec3::new(0.0, 0.2, 1.08), rot(-FRAC_PI_2))
        } else {
            (Vec3::ZERO, Quat::IDENTITY)
        };
        set(&mut transforms, rig.body, body_at, body_rot, blend);

        let swing = p.sin();
        let (hip_y, thigh_base, shin_base, lean, cycle) = match pose.stance {
            stance::CROUCH => (0.58, 1.25, -1.9, -0.2, 0.3),
            stance::PRONE => (HIP, 0.0, 0.0, 0.0, 0.15),
            _ => (HIP - (p.cos().abs() * 0.05 * k), 0.0, 0.0, -0.1 * k - if pose.sprinting { 0.18 } else { 0.0 }, 0.75),
        };
        set(&mut transforms, rig.hips, Vec3::Y * hip_y, Quat::IDENTITY, blend);
        for side in 0..2 {
            let s = if side == 0 { swing } else { -swing };
            let lift = if side == 0 { (p - 1.2).sin() } else { (p - 1.2 + PI).sin() };
            let thigh = thigh_base + s * cycle * k.min(1.2);
            let shin = shin_base - lift.max(0.0) * 1.1 * k.min(1.2) * if pose.stance == stance::STAND { 1.0 } else { 0.4 };
            set(&mut transforms, rig.thighs[side], Vec3::new(if side == 0 { -0.12 } else { 0.12 }, -0.02, 0.0), rot(thigh), blend);
            set(&mut transforms, rig.shins[side], Vec3::new(0.0, -THIGH, 0.0), rot(shin), blend);
        }

        // Upper body: lean into the run, aim where the player looks.
        let prone = pose.stance == stance::PRONE && !pose.dead;
        set(&mut transforms, rig.torso, Vec3::Y * 0.06, rot(lean) * Quat::from_rotation_y(swing * 0.08 * k.min(1.0)), blend);
        let carry = if pose.sprinting && !pose.aiming { -0.75 } else if pose.reloading { -0.45 } else { 0.0 };
        let correction = if prone { FRAC_PI_2 } else { -lean };
        let aim_pitch = if pose.dead { -0.9 } else { (pose.pitch + carry).clamp(-1.3, 1.3) + correction };
        let tilt = if pose.reloading { Quat::from_rotation_z(0.35) } else { Quat::IDENTITY };
        let aim_at = Vec3::Y * SHOULDER + if pose.aiming { Vec3::new(-0.04, 0.03, -0.04) } else { Vec3::ZERO };
        set(&mut transforms, rig.aim, aim_at, rot(aim_pitch) * tilt, blend);
        let head_pitch = if pose.dead { 0.0 } else { pose.pitch * 0.8 + correction };
        set(&mut transforms, rig.head, Vec3::Y * NECK, rot(head_pitch), blend);

        // Face: dead > hurt > angry > blink > smile.
        let current = pose.face.unwrap_or(if pose.dead {
            face::DEAD
        } else if rig.hurt > 0.0 {
            face::HURT
        } else if rig.angry > 0.0 || pose.aiming {
            face::ANGRY
        } else if rig.blink < 0.0 {
            face::BLINK
        } else {
            face::SMILE
        });
        for (index, entity) in rig.faces.iter().enumerate() {
            if let Ok(mut v) = visibility.get_mut(*entity) {
                let wanted = if index == current { Visibility::Inherited } else { Visibility::Hidden };
                if *v != wanted {
                    *v = wanted;
                }
            }
        }
    }
}

const WALK: f32 = 5.4;
