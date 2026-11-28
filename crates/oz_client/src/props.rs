//! How the breakable parts of a map look: doors on their hinges, window
//! glass, and street clutter - and what it looks like when they break.
//! The simulation (`oz_sim::breakables`) decides what breaks; this module
//! dresses the pieces and plays the moment they go.

use crate::{
    ClientSet,
    art::Art,
    fx::fragment,
    input::Controls,
    ui::Screen,
    view::WorldCamera,
};
use bevy::{pbr::NotShadowCaster, prelude::*};
use oz_core::worldgen::ClutterKind;
use oz_sim::{
    breakables::{ClutterPiece, DOOR_REACH, Door, Pane, door},
    combat::Solid,
    prelude::*,
};

#[derive(Resource)]
struct Glass {
    pane: Handle<StandardMaterial>,
    /// Tinted, for cars.
    tinted: Handle<StandardMaterial>,
}

/// The door the player could work right now, for the prompt.
#[derive(Resource, Default)]
pub struct DoorPrompt(pub Option<(usize, bool)>);

#[derive(Component)]
struct DoorPanel;

#[derive(Component)]
struct GlassPane;

#[derive(Component)]
struct ClutterModel;

pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DoorPrompt>()
            .add_systems(Startup, |mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>| {
                let glass = materials.add(StandardMaterial { base_color: Color::srgba(0.62, 0.86, 0.95, 0.3), alpha_mode: AlphaMode::Blend, perceptual_roughness: 0.1, ..default() });
                let tinted = materials.add(StandardMaterial { base_color: Color::srgba(0.2, 0.3, 0.36, 0.62), alpha_mode: AlphaMode::Blend, perceptual_roughness: 0.05, ..default() });
                commands.insert_resource(Glass { pane: glass, tinted });
            })
            .add_systems(Update, use_doors.in_set(ClientSet::Input).after(crate::input::ReadDevices).run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, (dress, follow_doors, breakage).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

fn clutter_model(kind: ClutterKind) -> BlockModel {
    match kind {
        ClutterKind::Bin => BlockModel::new()
            .cube(Paint::Teal, [0.0, 0.5, 0.0], [0.8, 1.0, 0.7])
            .cube(Paint::Dark, [0.0, 1.05, 0.0], [0.84, 0.1, 0.74])
            .cube(Paint::Steel, [0.0, 0.08, 0.0], [0.7, 0.16, 0.6]),
        ClutterKind::Barrel => BlockModel::new()
            .cylinder(Paint::Red, [0.0, 0.5, 0.0], [0.64, 1.0, 0.64])
            .cylinder(Paint::Dark, [0.0, 0.3, 0.0], [0.66, 0.06, 0.66])
            .cylinder(Paint::Dark, [0.0, 0.72, 0.0], [0.66, 0.06, 0.66])
            .cube(Paint::Gold, [0.0, 0.5, -0.33], [0.22, 0.22, 0.01]),
        ClutterKind::Bench => BlockModel::new()
            .cube(Paint::Wood, [0.0, 0.42, 0.0], [1.8, 0.08, 0.5])
            .cube(Paint::Wood, [0.0, 0.75, 0.22], [1.8, 0.4, 0.06])
            .cube(Paint::Dark, [-0.75, 0.2, 0.0], [0.08, 0.4, 0.5])
            .cube(Paint::Dark, [0.75, 0.2, 0.0], [0.08, 0.4, 0.5]),
        ClutterKind::Mailbox => BlockModel::new()
            .cube(Paint::Blue, [0.0, 0.75, 0.0], [0.5, 0.6, 0.5])
            .cube(Paint::Dark, [0.0, 0.22, 0.0], [0.12, 0.45, 0.12])
            .cube(Paint::Dark, [0.0, 0.88, -0.26], [0.3, 0.05, 0.02]),
        ClutterKind::Crates => BlockModel::new()
            .cube(Paint::Wood, [-0.2, 0.3, 0.0], [0.6, 0.6, 0.6])
            .cube(Paint::Wood, [0.3, 0.3, 0.1], [0.55, 0.55, 0.55])
            .cube(Paint::Wood, [0.0, 0.88, 0.0], [0.55, 0.55, 0.55]),
        ClutterKind::Car => car_model(Paint::Coral),
    }
}

/// A car's paint, by its place in the map's clutter: every street gets a mix.
pub fn car_paint(index: usize) -> Paint {
    [Paint::Coral, Paint::Teal, Paint::Navy, Paint::Cream, Paint::Mustard, Paint::Steel, Paint::Lino, Paint::Carpet][index % 8]
}

/// A parked car: a body with a bonnet and boot, a cabin of pillars and roof round open window
/// openings (the glass is its own breakable pane, see `oz_core::worldgen::CAR_WINDOWS`), seats,
/// dash and wheel inside, wheels with hubs, bumpers, lights, plates and mirrors.
pub fn car_model(body: Paint) -> BlockModel {
    let trim = Paint::Dark;
    let mut m = BlockModel::new()
        .cube(body, [0.0, 0.55, 0.0], [1.9, 0.56, 4.1])
        .cube(body, [0.0, 0.855, -1.5], [1.86, 0.06, 1.1])
        .cube(body, [0.0, 0.855, 1.72], [1.86, 0.06, 0.66])
        .cube(body, [0.0, 0.86, 0.28], [1.74, 0.05, 2.36])
        // Roof and pillars.
        .cube(body, [0.0, 1.37, 0.28], [1.68, 0.07, 1.9])
        .segment(body, [0.79, 0.86, -0.97], [0.77, 1.35, -0.72], 0.08)
        .segment(body, [-0.79, 0.86, -0.97], [-0.77, 1.35, -0.72], 0.08)
        .cube(body, [0.82, 1.11, 0.28], [0.08, 0.5, 0.13])
        .cube(body, [-0.82, 1.11, 0.28], [0.08, 0.5, 0.13])
        .segment(body, [0.8, 0.86, 1.46], [0.78, 1.35, 1.2], 0.09)
        .segment(body, [-0.8, 0.86, 1.46], [-0.78, 1.35, 1.2], 0.09)
        // Inside: seats, rear bench, dash and wheel.
        .cube(trim, [0.42, 0.98, -0.12], [0.5, 0.18, 0.5])
        .cube(trim, [-0.42, 0.98, -0.12], [0.5, 0.18, 0.5])
        .cube(trim, [0.42, 1.18, 0.16], [0.5, 0.48, 0.1])
        .cube(trim, [-0.42, 1.18, 0.16], [0.5, 0.48, 0.1])
        .cube(trim, [0.0, 0.98, 0.86], [1.4, 0.18, 0.5])
        .cube(trim, [0.0, 1.16, 1.12], [1.4, 0.44, 0.1])
        .cube(Paint::Slate, [0.0, 1.0, -0.78], [1.6, 0.16, 0.22])
        .cylinder(trim, [0.42, 1.1, -0.58], [0.3, 0.03, 0.3])
        .rotated([60.0, 0.0, 0.0])
        // Bumpers, grille, lights and plates (blank: no makes, no marques).
        .cube(trim, [0.0, 0.4, -2.08], [1.94, 0.16, 0.12])
        .cube(trim, [0.0, 0.4, 2.08], [1.94, 0.16, 0.12])
        .cube(trim, [0.0, 0.64, -2.06], [1.0, 0.16, 0.03])
        .cube(Paint::Glow, [0.66, 0.66, -2.06], [0.32, 0.12, 0.03])
        .cube(Paint::Glow, [-0.66, 0.66, -2.06], [0.32, 0.12, 0.03])
        .cube(Paint::Red, [0.68, 0.7, 2.06], [0.3, 0.12, 0.03])
        .cube(Paint::Red, [-0.68, 0.7, 2.06], [0.3, 0.12, 0.03])
        .cube(Paint::Cream, [0.0, 0.42, -2.15], [0.48, 0.12, 0.012])
        .cube(Paint::Cream, [0.0, 0.42, 2.15], [0.48, 0.12, 0.012])
        // Mirrors.
        .cube(trim, [0.99, 1.0, -0.68], [0.12, 0.09, 0.06])
        .cube(trim, [-0.99, 1.0, -0.68], [0.12, 0.09, 0.06]);
    // Door seams and handles, wheel arches, wheels with hubs.
    for side in [-1.0f32, 1.0] {
        let x = side * 0.951;
        m = m
            .cube(trim, [x, 0.56, -0.8], [0.008, 0.5, 0.012])
            .cube(trim, [x, 0.56, 0.3], [0.008, 0.5, 0.012])
            .cube(trim, [x, 0.56, 1.3], [0.008, 0.5, 0.012])
            .cube(Paint::Metal, [side * 0.958, 0.72, -0.62], [0.01, 0.03, 0.14])
            .cube(Paint::Metal, [side * 0.958, 0.72, 0.48], [0.01, 0.03, 0.14])
            .cube(trim, [side * 0.958, 0.34, 0.0], [0.01, 0.06, 2.0]);
        for z in [-1.3f32, 1.3] {
            m = m
                .cube(trim, [side * 0.952, 0.55, z], [0.01, 0.2, 0.86])
                .cylinder(trim, [side * 0.86, 0.32, z], [0.64, 0.26, 0.64])
                .rotated([0.0, 0.0, 90.0])
                .cylinder(Paint::Metal, [side * 0.995, 0.32, z], [0.36, 0.02, 0.36])
                .rotated([0.0, 0.0, 90.0]);
        }
    }
    m.cube(trim, [0.0, 0.3, 0.0], [1.7, 0.1, 3.6])
}

/// Give new pieces their looks.
#[allow(clippy::too_many_arguments)]
fn dress(
    mut commands: Commands,
    art: Res<Art>,
    glass: Res<Glass>,
    map: Res<Map>,
    mut meshes: ResMut<Assets<Mesh>>,
    doors: Query<Entity, Added<Door>>,
    panes: Query<(Entity, &Pane), Added<Pane>>,
    clutter: Query<(Entity, &ClutterPiece), Added<ClutterPiece>>,
) {
    for entity in &doors {
        commands.entity(entity).insert(Visibility::Inherited).with_children(|door| {
            door.spawn((DoorPanel, Mesh3d(art.primitive(Shape::Cube)), MeshMaterial3d(art.paint(Paint::Wood)), Transform::IDENTITY));
        });
    }
    for (entity, pane) in &panes {
        let Some(spec) = map.windows.get(pane.index) else { continue };
        let size = if spec.along_x { Vec3::new(spec.width, spec.height, 0.03) } else { Vec3::new(0.03, spec.height, spec.width) };
        commands.entity(entity).insert(Visibility::Inherited).with_children(|p| {
            let material = if spec.vehicle.is_some() { glass.tinted.clone() } else { glass.pane.clone() };
            p.spawn((GlassPane, Mesh3d(art.primitive(Shape::Cube)), MeshMaterial3d(material), Transform::from_translation(Vec3::Y * spec.height * 0.5).with_scale(size), NotShadowCaster));
        });
    }
    for (entity, piece) in &clutter {
        let Some(spec) = map.clutter.get(piece.index) else { continue };
        let model = if spec.kind == ClutterKind::Car { car_model(car_paint(piece.index)) } else { clutter_model(spec.kind) };
        let baked = art.bake(&model, &mut meshes);
        let model = commands.spawn((ClutterModel, Transform::from_rotation(Quat::from_rotation_y(if spec.turned { std::f32::consts::FRAC_PI_2 } else { 0.0 })), Visibility::Inherited, ChildOf(entity))).id();
        art.spawn(&mut commands, model, &baked);
        commands.entity(entity).insert(Visibility::Inherited);
    }
}

/// Door panels match their collision box: closed across the doorway, open against the wall.
fn follow_doors(doors: Query<(&Solid, &Children), With<Door>>, mut panels: Query<&mut Transform, With<DoorPanel>>) {
    for (solid, children) in &doors {
        for child in children.iter() {
            if let Ok(mut transform) = panels.get_mut(child) {
                let (at, scale) = (Vec3::Y * solid.half.y, solid.half * 2.0);
                if transform.scale != scale || transform.translation != at {
                    transform.translation = at;
                    transform.scale = scale;
                }
            }
        }
    }
}

/// When the shared state says something broke: hide it and throw the pieces.
#[allow(clippy::too_many_arguments)]
fn breakage(
    mut commands: Commands,
    art: Res<Art>,
    map: Res<Map>,
    state: Res<MatchState>,
    mut seen: Local<(Vec<u8>, Vec<u8>, Vec<u8>)>,
    doors: Query<(&Door, &Children)>,
    panes: Query<(&Pane, &Children)>,
    clutter: Query<(&ClutterPiece, &Children)>,
    mut visibility: Query<&mut Visibility, Or<(With<DoorPanel>, With<GlassPane>, With<ClutterModel>)>>,
    mut seed: Local<u32>,
) {
    if !state.is_changed() {
        return;
    }
    let mut random = || {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
    };
    let first = seen.0.len() != state.doors.len() || seen.1.len() != state.panes.len() || seen.2.len() != state.clutter.len();
    if first {
        *seen = (state.doors.clone(), state.panes.clone(), state.clutter.clone());
    }
    let hide = |children: &Children, visibility: &mut Query<&mut Visibility, Or<(With<DoorPanel>, With<GlassPane>, With<ClutterModel>)>>, gone: bool| {
        for child in children.iter() {
            if let Ok(mut v) = visibility.get_mut(child) {
                let wanted = if gone { Visibility::Hidden } else { Visibility::Inherited };
                if *v != wanted {
                    *v = wanted;
                }
            }
        }
    };
    for (d, children) in &doors {
        let broken = state.doors.get(d.index) == Some(&door::BROKEN);
        hide(children, &mut visibility, broken);
        if broken && !first && seen.0.get(d.index) != Some(&door::BROKEN)
            && let Some(spec) = map.doors.get(d.index) {
                for _ in 0..10 {
                    let at = spec.center + Vec3::new(random() * 0.6, 1.0 + random() * 0.9, random() * 0.6);
                    fragment(&mut commands, &art, Shape::Cube, Paint::Wood, at, Vec3::new(0.08, 0.5, 0.06), Vec3::new(random() * 3.0, 2.0 + random(), random() * 3.0), 2.5);
                }
            }
    }
    for (p, children) in &panes {
        let broken = state.panes.get(p.index) == Some(&1);
        hide(children, &mut visibility, broken);
        if broken && !first && seen.1.get(p.index) == Some(&0)
            && let Some(spec) = map.windows.get(p.index) {
                // A house pane breaks into shards; a car's safety glass into a shower of crumbs.
                let (pieces, size) = if spec.vehicle.is_some() { (18, 0.035) } else { (12, 0.05) };
                let along = if spec.along_x { Vec3::X } else { Vec3::Z };
                for _ in 0..pieces {
                    let at = spec.center + along * random() * spec.width * 0.45 + Vec3::Y * spec.height * (0.5 + random() * 0.45);
                    fragment(&mut commands, &art, Shape::Shard, if random() > 0.0 { Paint::Aether } else { Paint::White }, at, Vec3::splat(size * (1.0 + random() * 0.4)), Vec3::new(random() * 2.0, random() + 0.4, random() * 2.0), 1.6);
                }
            }
    }
    for (c, children) in &clutter {
        let wrecked = state.clutter.get(c.index) == Some(&0);
        hide(children, &mut visibility, wrecked);
        if wrecked && !first && seen.2.get(c.index).is_some_and(|hp| *hp > 0)
            && let Some(spec) = map.clutter.get(c.index) {
                let paint = match spec.kind {
                    ClutterKind::Bin => Paint::Teal,
                    ClutterKind::Barrel => Paint::Red,
                    ClutterKind::Mailbox => Paint::Blue,
                    ClutterKind::Car => car_paint(c.index),
                    _ => Paint::Wood,
                };
                let size = spec.half();
                for _ in 0..14 {
                    let at = spec.center() + Vec3::new(random() * size.x, random() * size.y, random() * size.z);
                    fragment(&mut commands, &art, Shape::Cube, if random() > 0.3 { paint } else { Paint::Dark }, at, Vec3::splat(0.12 + random().abs() * 0.15), Vec3::new(random() * 4.0, 3.0 + random() * 2.0, random() * 4.0), 3.0);
                }
                // What is left behind stays: a scorched heap.
                commands.spawn((
                    MatchEntity,
                    Mesh3d(art.primitive(Shape::Shard)),
                    MeshMaterial3d(art.paint(Paint::Dark)),
                    Transform::from_translation(spec.pos + Vec3::Y * 0.1).with_scale(Vec3::new(size.x * 0.9, 0.18, size.z * 0.9)),
                ));
            }
    }
    *seen = (state.doors.clone(), state.panes.clone(), state.clutter.clone());
}

/// Use (F) on a door you are looking at, within reach.
fn use_doors(
    controls: Res<Controls>,
    screen: Res<Screen>,
    map: Res<Map>,
    state: Res<MatchState>,
    nearby: Res<crate::stations::Nearby>,
    camera: Query<&Transform, With<WorldCamera>>,
    players: Query<&Player, With<LocalPlayer>>,
    mut prompt: ResMut<DoorPrompt>,
    mut orders: EventWriter<CommandEvent>,
) {
    let (Ok(eye), Ok(me)) = (camera.single(), players.single()) else { return };
    let forward = eye.forward().as_vec3();
    let best = map
        .doors
        .iter()
        .enumerate()
        .filter(|(i, _)| state.doors.get(*i).is_some_and(|s| *s != door::BROKEN))
        .map(|(i, d)| (i, d.center + Vec3::Y * 1.1 - eye.translation))
        .filter(|(_, to)| to.length() < DOOR_REACH && to.normalize_or_zero().dot(forward) > 0.55)
        .min_by(|a, b| a.1.length().total_cmp(&b.1.length()))
        .map(|(i, _)| (i, state.doors.get(i).is_some_and(|s| door::is_open(*s))));
    prompt.0 = best;
    if let Some((index, _)) = best
        && controls.interact
        && *screen == Screen::None
        && nearby.0.is_none()
    {
        orders.write(CommandEvent { player: me.id, command: PlayerCommand::Door(index as u16) });
    }
}
