//! Vision equipment, picked per class in the armory and switched on and off in a
//! match with N / D-pad left.
//!
//! * **Thermal:** enemy robots show as glowing hot silhouettes - through smoke,
//!   in shadow, at night - while the world goes cold and blue.
//! * **Night vision:** the world goes bright green, ideal for night maps,
//!   useless in daylight.

use crate::{
    ClientSet,
    art::Art,
    avatar::{PlayerRobot, Rig},
    fx::PlaySfx,
    input::Controls,
    ui::Screen,
};
use bevy::{pbr::NotShadowCaster, prelude::*};
use oz_sim::{player::vision, prelude::*};

#[derive(Resource, Default)]
pub struct NightVision {
    /// The green visor is down.
    pub on: bool,
    /// The thermal visor is down.
    pub thermal: bool,
}

#[derive(Component)]
struct ThermalGlow;

#[derive(Component)]
struct NightVisionTint;

#[derive(Component)]
struct ThermalTint;

pub struct OpticsPlugin;

impl Plugin for OpticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NightVision>()
            .add_systems(OnEnter(MatchPhase::Active), (spawn_tint, |mut nv: ResMut<NightVision>| *nv = NightVision::default()))
            .add_systems(Update, (toggle_night_vision, give_heat, show_heat, tint).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

/// The vision device the local player's class carries.
fn local_vision(players: &Query<&Player, With<LocalPlayer>>) -> u8 {
    players.single().map_or(vision::NONE, |p| p.vision)
}

fn toggle_night_vision(
    controls: Res<Controls>,
    screen: Res<Screen>,
    players: Query<&Player, With<LocalPlayer>>,
    health: Query<&Health, With<LocalPlayer>>,
    mut nv: ResMut<NightVision>,
    mut sounds: EventWriter<PlaySfx>,
) {
    let device = local_vision(&players);
    let alive = health.single().is_ok_and(|h| h.alive());
    if device == vision::NONE || !alive {
        if nv.on || nv.thermal {
            *nv = NightVision::default();
        }
        return;
    }
    // Swapped class: the other visor is not on the helmet any more.
    if (nv.on && device != vision::NIGHT) || (nv.thermal && device != vision::THERMAL) {
        *nv = NightVision::default();
    }
    if controls.night_vision && *screen == Screen::None {
        let down = !(nv.on || nv.thermal);
        nv.on = down && device == vision::NIGHT;
        nv.thermal = down && device == vision::THERMAL;
        sounds.write(PlaySfx::ui(if down { Sfx::Radar } else { Sfx::UiMove }));
    }
}

fn spawn_tint(mut commands: Commands) {
    commands.spawn((
        MatchEntity,
        NightVisionTint,
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        BackgroundColor(Color::srgba(0.15, 1.0, 0.35, 0.16)),
        Visibility::Hidden,
        GlobalZIndex(-45),
    ));
    commands.spawn((
        MatchEntity,
        ThermalTint,
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        BackgroundColor(Color::srgba(0.05, 0.12, 0.45, 0.42)),
        Visibility::Hidden,
        GlobalZIndex(-45),
    ));
}

/// Looking through a thermal scope counts as the thermal visor being down.
fn thermal_scope(view: &crate::view::ViewState) -> bool {
    view.scope_kind == 3 && view.scoped > 0.5
}

fn tint(nv: Res<NightVision>, view: Res<crate::view::ViewState>, mut green: Query<&mut Visibility, (With<NightVisionTint>, Without<ThermalTint>)>, mut blue: Query<&mut Visibility, (With<ThermalTint>, Without<NightVisionTint>)>) {
    let show = |visibility: &mut Visibility, on: bool| {
        let wanted = if on { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    };
    if let Ok(mut visibility) = green.single_mut() {
        show(&mut visibility, nv.on);
    }
    if let Ok(mut visibility) = blue.single_mut() {
        show(&mut visibility, nv.thermal || thermal_scope(&view));
    }
}

/// Every robot gets a hidden heat silhouette riding its body.
fn give_heat(mut commands: Commands, art: Res<Art>, robots: Query<&Rig, (Added<Rig>, With<PlayerRobot>)>) {
    for rig in &robots {
        commands.spawn((
            ThermalGlow,
            Mesh3d(art.primitive(Shape::Cube)),
            MeshMaterial3d(art.paint(Paint::Glow)),
            Transform::from_xyz(0.0, 1.0, 0.0).with_scale(Vec3::new(0.75, 1.85, 0.55)),
            Visibility::Hidden,
            NotShadowCaster,
            ChildOf(rig.body),
        ));
    }
}

/// Glow the enemy silhouettes while the thermal visor is down.
fn show_heat(
    nv: Res<NightVision>,
    view: Res<crate::view::ViewState>,
    rules: Res<Rules>,
    local: Query<&Player, With<LocalPlayer>>,
    robots: Query<(&Rig, &Player, &Health), With<PlayerRobot>>,
    mut glows: Query<(&ChildOf, &mut Visibility), With<ThermalGlow>>,
) {
    let me = local.single().ok();
    let thermal = nv.thermal || thermal_scope(&view);
    for (parent, mut visibility) in &mut glows {
        let hot = thermal
            && robots
                .iter()
                .find(|(rig, ..)| rig.body == parent.parent())
                .is_some_and(|(_, player, health)| health.alive() && me.is_some_and(|me| player.id != me.id && rules.pvp && player.team != me.team));
        let wanted = if hot { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}
