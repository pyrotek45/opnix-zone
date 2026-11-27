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
use std::f32::consts::{FRAC_PI_2, PI, TAU};

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
pub(crate) struct RobotMeshes {
    pub(crate) hips: Baked,
    pub(crate) thigh: Baked,
    pub(crate) shin: Baked,
    pub(crate) torso: Baked,
    pub(crate) head: Baked,
    /// Left and right arm, separately so either can be shot off.
    pub(crate) arms: [Baked; 2],
    pub(crate) faces: Vec<Baked>,
}

#[derive(Resource)]
pub struct RobotParts {
    pub(crate) teams: Vec<RobotMeshes>,
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
    /// On a ladder, and how fast it is climbing (m/s).
    pub climbing: bool,
    pub climb_rate: f32,
    pub dead: bool,
    /// Leaning around cover, -1 (left) to 1 (right), smoothed.
    pub lean: f32,
    /// In cover: how far to turn the body (radians, relative to where the player looks)
    /// so its back is to the wall.
    pub cover: Option<f32>,
    /// Firing blind from cover: the gun held up over the top (or round the edge).
    pub blind: bool,
    /// Force a face (showroom); otherwise the face follows events.
    pub face: Option<usize>,
}

/// Joint handles and animation state for one robot.
#[derive(Component)]
pub struct Rig {
    pub(crate) body: Entity,
    pub(crate) hips: Entity,
    pub(crate) thighs: [Entity; 2],
    pub(crate) shins: [Entity; 2],
    pub(crate) torso: Entity,
    pub(crate) head: Entity,
    pub(crate) aim: Entity,
    pub(crate) arms: [Entity; 2],
    pub(crate) gun: Entity,
    pub(crate) team: u8,
    /// Parts not drawn: limbs shot off, or blown away on death (see `HIDE_*`).
    pub hidden: u8,
    pub(crate) faces: [Entity; face::COUNT],
    pub(crate) phase: f32,
    pub(crate) dead_for: f32,
    pub(crate) blink: f32,
    pub angry: f32,
    pub hurt: f32,
    pub(crate) last_hp: f32,
    pub(crate) last_pos: Vec3,
    /// The weapon (index, parts) currently in its hands.
    pub(crate) held: Option<(u8, [u8; SLOT_COUNT])>,
    /// Smoothed stance weights (0..1) and how much it is moving: everything blends.
    pub(crate) crouch: f32,
    pub(crate) prone: f32,
    pub(crate) moving: f32,
    /// Hip height on a spring, so dropping and popping up have a little weight to them.
    pub(crate) hip: Spring,
    /// Gun recoil on a spring: each shot kicks it, it settles with a slight bounce.
    pub(crate) kick: Spring,
    /// Seconds into the current reload; the magazine slap kicks once.
    pub(crate) reload_t: f32,
    pub(crate) slapped: bool,
    pub(crate) reload: f32,
    /// Which way it falls when it dies (robot-local, x right / z back) and how hard.
    pub(crate) fall: Vec3,
    /// Footfalls so far, so each new one can be heard.
    pub(crate) steps: i32,
    /// Holding a grenade with the pin out, seconds since it was thrown, and since a melee strike.
    pub(crate) cook: bool,
    /// Ladder: how much of the climbing pose is showing, and where in the hand-over-hand cycle.
    pub(crate) climb: f32,
    /// Hunkered behind cover (0..1), and arms up for blind fire (0..1).
    pub(crate) hunker: f32,
    pub(crate) blind: f32,
    pub(crate) climb_phase: f32,
    /// The body's current turn away from the view (cover), eased.
    pub(crate) turn: f32,
    pub(crate) throw_t: f32,
    pub(crate) melee_t: f32,
}

/// A damped spring for procedural motion.
#[derive(Clone, Copy, Default)]
pub(crate) struct Spring {
    value: f32,
    velocity: f32,
}

impl Spring {
    /// `stiffness` sets the speed, `damping` below 1 overshoots a little.
    fn update(&mut self, target: f32, stiffness: f32, damping: f32, dt: f32) -> f32 {
        let c = 2.0 * stiffness.sqrt() * damping;
        // A few small steps keep stiff springs stable at low frame rates.
        let steps = 4;
        let h = dt / steps as f32;
        for _ in 0..steps {
            let accel = stiffness * (target - self.value) - c * self.velocity;
            self.velocity += accel * h;
            self.value += self.velocity * h;
        }
        self.value
    }
}

/// Marks a robot that belongs to a player in the world.
#[derive(Component)]
pub struct PlayerRobot;

/// The local player's own body, seen only in third person.
#[derive(Component)]
pub struct OwnRobot;

fn show_own_robot(view: Res<crate::view::ViewState>, mut robot: Query<&mut Visibility, With<OwnRobot>>) {
    let wanted = if view.third > 0.4 { Visibility::Inherited } else { Visibility::Hidden };
    for mut visibility in &mut robot {
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

pub struct AvatarPlugin;

/// Label for the system that poses every robot.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct AnimateRigs;

impl Plugin for AvatarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GunCache>().add_systems(Startup, bake_robots).add_systems(
            Update,
            ((attach_to_players, pose_players, react_to_shots, hold_weapons, show_own_robot, shine_torches).run_if(in_state(MatchPhase::Active)), animate_rigs.in_set(AnimateRigs)).chain().in_set(ClientSet::Present),
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
                m = m.segment(paint, [x - 0.03, 0.23, z], [x + 0.03, 0.17, z], 0.016).segment(paint, [x + 0.03, 0.23, z], [x - 0.03, 0.17, z], 0.016);
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

/// The glowing team colour: unlit, so it reads in shade, at dusk and at range.
pub fn team_light(team: u8) -> Paint {
    match team {
        0 => Paint::Aether,
        1 => Paint::Red,
        _ => Paint::Glow,
    }
}

fn head_model(team: Paint, light: Paint) -> BlockModel {
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
        .shard(light, [-0.16, 0.5, 0.02], [0.035, 0.035, 0.035])
        .shard(light, [0.15, 0.53, 0.02], [0.035, 0.035, 0.035])
        .cube(Paint::Dark, [0.0, 0.315, 0.0], [0.12, 0.02, 0.08])
        // Team light bars down both sides of the set.
        .cube(light, [0.193, 0.17, 0.0], [0.006, 0.2, 0.18])
        .cube(light, [-0.193, 0.17, 0.0], [0.006, 0.2, 0.18])
        // A team marker floating over the head, readable across the map.
        .shard(light, [0.0, 0.66, 0.0], [0.035, 0.045, 0.035])
}

fn torso_model(team: Paint, light: Paint) -> BlockModel {
    BlockModel::new()
        .cube(Paint::Dark, [0.0, 0.04, 0.0], [0.34, 0.14, 0.22])
        .cube(team, [0.0, 0.3, 0.0], [0.46, 0.44, 0.28])
        .cube(Paint::Steel, [0.0, 0.34, -0.145], [0.34, 0.22, 0.02])
        // Chest band and core in the team light.
        .cube(light, [0.0, 0.2, -0.146], [0.44, 0.035, 0.02])
        .shard(light, [0.0, 0.36, -0.16], [0.05, 0.05, 0.02])
        .cube(team, [0.29, 0.48, 0.0], [0.15, 0.12, 0.26])
        .cube(team, [-0.29, 0.48, 0.0], [0.15, 0.12, 0.26])
        .cube(light, [0.37, 0.48, 0.0], [0.012, 0.06, 0.2])
        .cube(light, [-0.37, 0.48, 0.0], [0.012, 0.06, 0.2])
        .cube(Paint::Dark, [0.0, 0.3, 0.19], [0.32, 0.36, 0.12])
        .cube(light, [0.0, 0.36, 0.252], [0.2, 0.035, 0.006])
        .cube(light, [0.0, 0.3, 0.252], [0.2, 0.035, 0.006])
        .cube(light, [0.0, 0.24, 0.252], [0.2, 0.035, 0.006])
}

fn arm_model(team: Paint, light: Paint, right: bool) -> BlockModel {
    let (shoulder, elbow, hand) = if right { ([0.27, 0.02, 0.0], [0.25, -0.2, -0.1], [GRIP.x, GRIP.y, GRIP.z + 0.03]) } else { ([-0.27, 0.02, 0.0], [-0.2, -0.17, -0.3], [0.06, -0.08, -0.66]) };
    BlockModel::new().segment(team, shoulder, elbow, 0.1).segment(Paint::Dark, elbow, hand, 0.085).shard(light, elbow, [0.07, 0.07, 0.07]).cube(Paint::Dark, hand, [0.09, 0.09, 0.09])
}

fn thigh_model(team: Paint) -> BlockModel {
    BlockModel::new().cube(team, [0.0, -THIGH * 0.5, 0.0], [0.16, THIGH, 0.18]).cube(Paint::Steel, [0.0, -THIGH + 0.02, -0.095], [0.13, 0.1, 0.03])
}

fn shin_model(light: Paint) -> BlockModel {
    BlockModel::new().cube(Paint::Dark, [0.0, -0.2, 0.0], [0.14, 0.4, 0.16]).cube(light, [0.0, -0.2, -0.081], [0.05, 0.22, 0.006]).cube(Paint::Steel, [0.0, -0.44, -0.05], [0.17, 0.08, 0.29])
}

fn bake_robots(mut commands: Commands, art: Res<Art>, mut meshes: ResMut<Assets<Mesh>>) {
    let teams = (0..3u8)
        .map(|team| {
            let paint = team_paint(team);
            let light = team_light(team);
            RobotMeshes {
                hips: art.bake(&BlockModel::new().cube(Paint::Dark, [0.0, 0.0, 0.0], [0.36, 0.16, 0.22]).cube(light, [0.0, 0.0, -0.112], [0.3, 0.03, 0.006]), &mut meshes),
                thigh: art.bake(&thigh_model(paint), &mut meshes),
                shin: art.bake(&shin_model(light), &mut meshes),
                torso: art.bake(&torso_model(paint, light), &mut meshes),
                head: art.bake(&head_model(paint, light), &mut meshes),
                arms: [art.bake(&arm_model(paint, light, false), &mut meshes), art.bake(&arm_model(paint, light, true), &mut meshes)],
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
    let aim = joint(commands, art, torso, Vec3::Y * SHOULDER, None);
    let arms = [joint(commands, art, aim, Vec3::ZERO, Some(&m.arms[0])), joint(commands, art, aim, Vec3::ZERO, Some(&m.arms[1]))];
    let gun_root = joint(commands, art, aim, GRIP, gun);
    Rig {
        body,
        hips,
        thighs,
        shins,
        torso,
        head,
        aim,
        arms,
        gun: gun_root,
        faces,
        team,
        hidden: 0,
        phase: 0.0,
        dead_for: 0.0,
        blink: 2.0,
        angry: 0.0,
        hurt: 0.0,
        last_hp: 0.0,
        last_pos: Vec3::NAN,
        held: None,
        crouch: 0.0,
        prone: 0.0,
        moving: 0.0,
        hip: Spring { value: HIP, velocity: 0.0 },
        kick: Spring::default(),
        reload_t: 0.0,
        slapped: false,
        reload: 0.0,
        fall: Vec3::Z,
        steps: 0,
        cook: false,
        throw_t: 9.0,
        melee_t: 9.0,
        turn: 0.0,
        climb: 0.0,
        climb_phase: 0.0,
        hunker: 0.0,
        blind: 0.0,
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
    added: Query<(Entity, &Player, &Transform, Has<LocalPlayer>), Added<Player>>,
) {
    if content.is_added() {
        cache.clear();
    }
    for (entity, player, transform, local) in &added {
        let mut rig = spawn_robot(&mut commands, &art, &parts, entity, player.team, None);
        rig.last_pos = transform.translation;
        commands.entity(entity).insert((rig, Pose::default(), PlayerRobot));
        if local {
            // Your own robot only shows when the camera is outside your head (cover).
            commands.entity(entity).insert((OwnRobot, Visibility::Hidden));
        }
    }
}

/// A robot's shoulder lamp: the beam other players see coming round a corner at night.
#[derive(Component)]
pub struct Torch {
    owner: Entity,
}

/// Switches every other robot's lamp to follow its owner's flashlight toggle. Lights only
/// show in the dark; in daylight nobody would notice.
fn shine_torches(
    mut commands: Commands,
    art: Res<Art>,
    state: Res<MatchState>,
    added: Query<(Entity, &Rig), (Added<Rig>, With<PlayerRobot>, Without<LocalPlayer>)>,
    owners: Query<(&PlayerInput, &Health)>,
    mut torches: Query<(&Torch, Option<&mut SpotLight>, &mut Visibility)>,
) {
    for (owner, rig) in &added {
        commands.spawn((
            Torch { owner },
            SpotLight { color: Color::srgb(1.0, 0.93, 0.8), intensity: 0.0, range: 40.0, outer_angle: 0.45, inner_angle: 0.25, shadows_enabled: false, ..default() },
            Transform::from_xyz(0.16, 0.06, -0.12),
            Visibility::Hidden,
            ChildOf(rig.aim),
        ));
        // The lens: a bright dot that gives the light away even when the beam points elsewhere.
        commands.spawn((
            Torch { owner },
            Mesh3d(art.primitive(Shape::Cube)),
            MeshMaterial3d(art.paint(Paint::Glow)),
            Transform::from_xyz(0.16, 0.06, -0.13).with_scale(Vec3::new(0.05, 0.05, 0.02)),
            Visibility::Hidden,
            bevy::pbr::NotShadowCaster,
            ChildOf(rig.aim),
        ));
    }
    let night = (1.0 - state.daylight.clamp(0.0, 1.0) * 2.0).clamp(0.0, 1.0);
    for (torch, light, mut visibility) in &mut torches {
        let on = night > 0.0 && owners.get(torch.owner).is_ok_and(|(input, health)| input.light && health.alive());
        let wanted = if on { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
        if let Some(mut light) = light {
            light.intensity = if on { 1_200_000.0 * night } else { 0.0 };
        }
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

fn pose_players(
    time: Res<Time>,
    mut robots: Query<(&mut Rig, &mut Pose, &Transform, &Player, &PlayerInput, &Health, Has<LocalPlayer>, Option<&Mover>), With<PlayerRobot>>,
    mut sounds: EventWriter<crate::fx::PlaySfx>,
    map: Res<Map>,
) {
    let dt = time.delta_secs().max(1e-4);
    for (mut rig, mut pose, transform, player, input, health, local, mover) in &mut robots {
        // Everyone else's feet are heard where they fall: listen for people coming.
        let footfalls = (rig.phase / PI).floor() as i32;
        if footfalls != rig.steps {
            rig.steps = footfalls;
            if !local && health.alive() && pose.speed.abs() > 0.8 {
                // Loud enough to place: sprinters carry far, crouch-walkers barely at all.
                let (sfx, volume) = match pose.stance {
                    stance::PRONE => (Sfx::Crawl, 0.5),
                    stance::CROUCH => (Sfx::StepSoft, 0.6),
                    _ if pose.sprinting => (Sfx::StepHeavy, 1.6),
                    _ => (Sfx::Step, 1.2),
                };
                sounds.write(crate::fx::PlaySfx::new(sfx, Some(transform.translation)).quiet(volume));
            }
        }
        let moved = if rig.last_pos.is_finite() { transform.translation - rig.last_pos } else { Vec3::ZERO };
        rig.last_pos = transform.translation;
        // Climbing: known for our own body; for others, rising steadily right at a ladder.
        let rising = moved.y / dt;
        let at = transform.translation;
        let on_ladder = map.ladders.iter().any(|l| (Vec2::new(at.x, at.z) - (Vec2::new(l.foot.x, l.foot.z) + l.out * 0.45)).length() < 0.7 && at.y > l.foot.y - 0.2 && at.y < l.top);
        pose.climbing = health.alive() && if local { mover.is_some_and(|m| m.climbing) } else { on_ladder && rising > 0.5 };
        pose.climb_rate += (rising.max(0.0) - pose.climb_rate) * (1.0 - (-dt * 8.0).exp());
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
        let lean = if input.lean != 0 { input.lean } else { flags::lean(player.flags) };
        let lean = if pose.stance == stance::PRONE || pose.dead { 0.0 } else { lean as f32 };
        pose.lean += (lean - pose.lean) * (1.0 - (-dt * 12.0).exp());
        let facing = transform.rotation.to_euler(EulerRot::YXZ).0;
        pose.cover = oz_sim::player::cover_yaw(input.cover).filter(|_| !pose.dead).map(|yaw| (yaw - facing + PI).rem_euclid(TAU) - PI);
        pose.blind = input.held(oz_sim::player::buttons::BLIND) && !pose.dead;
        if health.hp < rig.last_hp - 0.5 {
            rig.hurt = 0.6;
        }
        rig.last_hp = health.hp;
    }
}

/// Robots scowl while shooting, wind up and throw grenades, and swing their guns in melee.
fn react_to_shots(mut events: EventReader<FxEvent>, mut robots: Query<(&mut Rig, &Player)>) {
    for event in events.read() {
        let by = match event {
            FxEvent::Shot { by, .. } | FxEvent::Gear { by, .. } | FxEvent::Melee { by, .. } => *by,
            _ => continue,
        };
        let Some((mut rig, _)) = robots.iter_mut().find(|(_, player)| player.id == by) else {
            continue;
        };
        match event {
            FxEvent::Shot { .. } => {
                rig.angry = 1.2;
                // Shove the gun back; the spring brings it home.
                rig.kick.velocity += 7.0;
            }
            FxEvent::Gear { stage, .. } => {
                rig.cook = *stage == 0;
                if *stage == 1 {
                    rig.throw_t = 0.0;
                }
            }
            FxEvent::Melee { .. } => {
                rig.melee_t = 0.0;
                rig.angry = 1.0;
            }
            _ => {}
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
///
/// Everything is procedural: stance changes blend through weights that ease
/// in, the hips ride a spring so crouching and popping up have weight, gait
/// cycles scale with speed, the reload is a sequence of offsets driven by a
/// timer, and recoil is an impulse on a bouncy spring.
pub fn animate_rigs(time: Res<Time>, mut rigs: Query<(&mut Rig, &Pose)>, mut transforms: Query<&mut Transform, Without<Rig>>, mut visibility: Query<&mut Visibility, Without<Rig>>) {
    let dt = time.delta_secs().min(0.05);
    let blend = 1.0 - (-dt * 22.0).exp();
    let rot = |x: f32| Quat::from_rotation_x(x);
    let ease = |t: f32| {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    for (mut rig, pose) in &mut rigs {
        // Gait: long, unhurried strides - one full cycle (two steps) per ~3.3 m when upright,
        // shorter for a crouch-walk and a crawl - running backwards when retreating.
        let k = (pose.speed.abs() / WALK).min(1.6);
        let cadence = match pose.stance {
            stance::PRONE => 3.6,
            stance::CROUCH => 2.6,
            _ => 1.9,
        };
        rig.phase += pose.speed * dt * cadence;
        rig.throw_t += dt;
        rig.melee_t += dt;
        rig.climb += ((pose.climbing as u8 as f32) - rig.climb) * (1.0 - (-dt * 10.0).exp());
        rig.climb_phase += pose.climb_rate * dt * 5.5;
        let p = rig.phase;
        rig.dead_for = if pose.dead { rig.dead_for + dt } else { 0.0 };
        rig.angry = (rig.angry - dt).max(0.0);
        rig.hurt = (rig.hurt - dt).max(0.0);
        rig.blink -= dt;
        if rig.blink < -0.12 {
            rig.blink = 2.0 + (p * 7.3).sin().abs() * 2.5;
        }

        // Stance weights ease toward the target over about a fifth of a second.
        let towards = |current: f32, target: f32, rate: f32| current + (target - current) * (1.0 - (-dt * rate).exp());
        rig.crouch = towards(rig.crouch, (pose.stance == stance::CROUCH) as u8 as f32, 11.0);
        rig.prone = towards(rig.prone, (pose.stance == stance::PRONE) as u8 as f32, 7.0);
        rig.moving = towards(rig.moving, (k * 3.0).min(1.0), 8.0);
        let (c, pr) = (rig.crouch, rig.prone);
        let st = (1.0 - c - pr).max(0.0);
        let moving = rig.moving;

        // Whole body: prone lies forward, dead falls backward.
        let (body_at, body_rot) = if pose.dead {
            // Topple the way the killing blow pushed: fast, a small bounce on landing, then still.
            let t = rig.dead_for;
            let fall = ease(t / 0.4) + (t * 18.0).sin() * 0.08 * (-(t - 0.4).max(0.0) * 8.0).exp() * (t > 0.4) as u8 as f32;
            let dir = Vec3::new(rig.fall.x, 0.0, rig.fall.z).normalize_or(Vec3::Z);
            let axis = Vec3::Y.cross(dir).normalize_or(Vec3::X);
            let slump = Vec3::new(0.0, 0.18, 0.0) + dir * 0.35;
            (slump * fall, Quat::from_axis_angle(axis, FRAC_PI_2 * fall))
        } else {
            (Vec3::new(0.0, 0.2, 1.08) * pr, Quat::IDENTITY.slerp(rot(-FRAC_PI_2), pr))
        };
        // Cover: back to the wall while waiting; turn out to face the shot when aiming or firing.
        // Blind fire keeps the back to the wall: only the arms come up and round.
        let wanted_turn = match pose.cover {
            Some(turn) if (!pose.aiming && rig.angry < 0.8) || pose.blind => turn,
            _ => 0.0,
        };
        let gap = (wanted_turn - rig.turn + PI).rem_euclid(TAU) - PI;
        rig.turn += gap * (1.0 - (-dt * 10.0).exp());
        let body_rot = Quat::from_rotation_y(rig.turn) * body_rot;
        set(&mut transforms, rig.body, body_at, body_rot, blend);

        // Hunkered down behind cover: lower and folded over, so the head stays under the top
        // of a waist-high wall (the hit spheres in `weapons` drop the same way).
        let hunker_target = (pose.cover.is_some() && pose.stance == stance::CROUCH && !pose.aiming && !pose.dead) as u8 as f32;
        rig.hunker = towards(rig.hunker, hunker_target, 9.0);
        let hunker = rig.hunker * c;
        // Hips: standing bobs with the stride; crouch-walking stays low and level; kneeling is lowest.
        let swing = p.sin();
        let stand_hip = HIP - p.cos().abs() * 0.05 * k;
        let crouch_hip = 0.5 + 0.06 * moving - p.cos().abs() * 0.02 * k - 0.1 * rig.hunker;
        let hip_target = st * stand_hip + c * crouch_hip + pr * HIP;
        let hip_y = rig.hip.update(hip_target, 260.0, 0.62, dt);
        set(&mut transforms, rig.hips, Vec3::Y * hip_y, Quat::IDENTITY, 1.0);

        for side in 0..2 {
            let sign = if side == 0 { 1.0 } else { -1.0 };
            let s = swing * sign;
            let lift = if side == 0 { (p - 1.2).sin() } else { (p - 1.2 + PI).sin() }.max(0.0);
            // Standing walk / run: a long reach forward, the trailing knee folding high.
            let stand = (s * (0.78 * k).min(0.95), -lift * (1.05 * k).min(1.25));
            // Crouch-walk: knees well bent, short quick steps; blends into a kneel when still.
            let crouch_walk = (0.85 + s * 0.42, -1.35 - lift * 0.55);
            // Kneeling: worked out from the hip height so the front foot is flat on the
            // ground and the back knee rests on it (no sinking into the floor).
            let kneel = kneel_leg(rig.hip.value, side == 0);
            let crouch = (kneel.0 + (crouch_walk.0 - kneel.0) * moving, kneel.1 + (crouch_walk.1 - kneel.1) * moving);
            // Prone: legs out straight with a slow crawl.
            let prone = (s * 0.15 * k.min(1.0), -lift * 0.3 * k.min(1.0));
            let thigh = st * stand.0 + c * crouch.0 + pr * prone.0;
            let shin = st * stand.1 + c * crouch.1 + pr * prone.1;
            // On a ladder: knees come up one after the other, in time with the hands.
            let step = (rig.climb_phase + side as f32 * PI).sin();
            let (thigh, shin) = (thigh + (0.75 + 0.55 * step - thigh) * rig.climb, shin + (-1.0 - 0.5 * step.max(0.0) - shin) * rig.climb);
            set(&mut transforms, rig.thighs[side], Vec3::new(-0.12 * sign, -0.02, 0.0), rot(thigh), blend);
            set(&mut transforms, rig.shins[side], Vec3::new(0.0, -THIGH, 0.0), rot(shin), blend);
        }

        // Reload: tip the gun in, drop the magazine, slap a fresh one home, bring it back up.
        if pose.reloading {
            rig.reload_t += dt;
            if rig.reload_t > 0.75 && !rig.slapped {
                rig.slapped = true;
                rig.kick.velocity -= 5.0;
            }
        } else {
            rig.reload_t = 0.0;
            rig.slapped = false;
        }
        rig.reload = towards(rig.reload, pose.reloading as u8 as f32, 9.0);
        let r = rig.reload;
        let fumble = if pose.reloading { (rig.reload_t * 9.0).sin() * 0.04 * ease((rig.reload_t - 0.2) / 0.2) * (1.0 - ease((rig.reload_t - 0.7) / 0.1)) } else { 0.0 };
        let kick = rig.kick.update(0.0, 420.0, 0.35, dt);

        // Upper body: lean into the run, peek out of cover, aim where the player looks.
        let prone = pr > 0.5 && !pose.dead;
        let torso_lean = st * (-0.1 * k - if pose.sprinting { 0.18 } else { 0.0 }) + c * (-0.24 - 0.1 * moving) - 0.32 * hunker;
        // Leaning out to aim rolls the whole torso; blind fire (leaning without aiming) keeps
        // the body in and only pushes the gun round the corner.
        let roll = Quat::from_rotation_z(-pose.lean * if pose.aiming { 0.3 } else { 0.06 });
        let twist = Quat::from_rotation_y(swing * 0.08 * k.min(1.0) * st);
        set(&mut transforms, rig.torso, Vec3::Y * 0.06, roll * rot(torso_lean) * twist, blend);
        let carry = if pose.sprinting && !pose.aiming { -0.75 } else { 0.0 } - 0.5 * r;
        let correction = pr * FRAC_PI_2 - (1.0 - pr) * torso_lean;
        let aim_pitch = if pose.dead { -0.9 } else { (pose.pitch + carry).clamp(-1.3, 1.3) + correction + kick * 0.12 };
        let aim_pitch = aim_pitch + (-1.35 - aim_pitch) * rig.climb;
        let tilt = Quat::from_rotation_z(0.45 * r + fumble * 2.0);
        let melee = if rig.melee_t < 0.42 { (rig.melee_t / 0.42 * PI).sin() } else { 0.0 };
        // Blind fire over low cover: both arms up, the gun held over the top.
        rig.blind = towards(rig.blind, (pose.blind && pose.lean.abs() < 0.3) as u8 as f32, 14.0);
        let aim_at = Vec3::Y * SHOULDER
            + if pose.aiming { Vec3::new(-0.04, 0.03, -0.04) } else { Vec3::new(pose.lean * 0.4, 0.0, 0.0) }
            + Vec3::new(0.0, 0.62, 0.12) * rig.blind
            + Vec3::new(0.0, 0.05, -0.3) * melee
            + Vec3::new(0.0, -0.05 * r + fumble, 0.07 * kick.max(0.0) - 0.03 * kick.min(0.0));
        // Firing blind the arms swing round to point the gun where the player looks, over
        // the top or round the edge, while the body stays turned to the wall.
        let counter = if pose.blind { Quat::from_rotation_y(-rig.turn) } else { Quat::IDENTITY };
        set(&mut transforms, rig.aim, aim_at, counter * rot(aim_pitch + 0.4 * melee) * Quat::from_rotation_y(0.5 * melee) * tilt, if prone { blend } else { 1.0 - (-dt * 30.0).exp() });
        // The left arm: wound back over the shoulder while a grenade cooks, then an overarm throw.
        let arm = if rig.cook {
            rot(2.2) * Quat::from_rotation_z(0.3)
        } else if rig.throw_t < 0.45 {
            let k = ease(rig.throw_t / 0.45);
            rot(2.2 - 3.0 * k) * Quat::from_rotation_z(0.3 * (1.0 - k))
        } else {
            Quat::IDENTITY
        };
        // On a ladder both hands reach up rung over rung; the gun hangs on its sling.
        let reach = |side: f32| rot(2.5 + 0.45 * (rig.climb_phase + side * PI).sin());
        let arm = arm.slerp(reach(0.0), rig.climb);
        set(&mut transforms, rig.arms[0], Vec3::ZERO, arm, blend);
        if rig.climb > 0.01 {
            set(&mut transforms, rig.arms[1], Vec3::ZERO, Quat::IDENTITY.slerp(reach(1.0), rig.climb), blend);
        } else {
            set(&mut transforms, rig.arms[1], Vec3::ZERO, Quat::IDENTITY, blend);
        }
        // The head follows the aim, and glances down at the gun while reloading.
        let head_pitch = if pose.dead { 0.0 } else { pose.pitch * 0.8 + correction - 0.35 * r - 0.35 * hunker };
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

/// Thigh and knee angles for a kneel at hip height `hip`: the front leg (`front`) puts
/// its foot flat on the ground a little ahead; the back leg rests its knee on the ground
/// with the shin lying back along it. Two-bone IK in the leg's own plane.
fn kneel_leg(hip: f32, front: bool) -> (f32, f32) {
    const SHIN: f32 = 0.46;
    // The thigh joint sits just under the hips.
    let drop = (hip - 0.02).max(0.2);
    if front {
        // Foot (sole) on the ground, a little forward of the hip.
        let (x, y) = (0.3f32, drop);
        let d = (x * x + y * y).sqrt().clamp(0.05, THIGH + SHIN - 1e-3);
        let toward = x.atan2(y);
        let at_hip = ((THIGH * THIGH + d * d - SHIN * SHIN) / (2.0 * THIGH * d)).clamp(-1.0, 1.0).acos();
        let at_foot = ((SHIN * SHIN + d * d - THIGH * THIGH) / (2.0 * SHIN * d)).clamp(-1.0, 1.0).acos();
        let thigh = toward + at_hip;
        let shin = toward - at_foot;
        (thigh, shin - thigh)
    } else {
        // Knee on the ground (its pad is ~0.06 thick), shin flat behind.
        let thigh = -((drop - 0.06) / THIGH).clamp(-1.0, 1.0).acos();
        (thigh, -FRAC_PI_2 - thigh)
    }
}

const WALK: f32 = 5.4;
