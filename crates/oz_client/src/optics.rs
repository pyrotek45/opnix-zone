//! Vision perks.
//!
//! * **Thermal optics:** while you aim down sights, enemy robots show as
//!   glowing hot silhouettes - through smoke, in shadow, at night.
//! * **Night vision:** a visor you switch on and off (N / D-pad left): the
//!   world goes bright green, ideal for night maps, useless in daylight.

use crate::{
    ClientSet,
    art::Art,
    avatar::{PlayerRobot, Rig},
    fx::PlaySfx,
    input::Controls,
    ui::Screen,
    view::ViewState,
};
use bevy::{pbr::NotShadowCaster, prelude::*};
use oz_core::content::PerkEffect;
use oz_sim::prelude::*;

#[derive(Resource, Default)]
pub struct NightVision {
    pub on: bool,
}

#[derive(Component)]
struct ThermalGlow;

#[derive(Component)]
struct NightVisionTint;

pub struct OpticsPlugin;

impl Plugin for OpticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NightVision>()
            .add_systems(
                OnEnter(MatchPhase::Active),
                (spawn_tint, |mut nv: ResMut<NightVision>| nv.on = false),
            )
            .add_systems(
                Update,
                (toggle_night_vision, give_heat, show_heat, tint)
                    .in_set(ClientSet::Present)
                    .run_if(in_state(MatchPhase::Active)),
            );
    }
}

fn local_perk(
    content: &Content,
    players: &Query<&Player, With<LocalPlayer>>,
) -> Option<PerkEffect> {
    players
        .single()
        .ok()
        .and_then(|p| oz_sim::player::perk(content, p))
}

fn toggle_night_vision(
    controls: Res<Controls>,
    screen: Res<Screen>,
    content: Res<Content>,
    players: Query<&Player, With<LocalPlayer>>,
    health: Query<&Health, With<LocalPlayer>>,
    mut nv: ResMut<NightVision>,
    mut sounds: EventWriter<PlaySfx>,
) {
    let has = matches!(
        local_perk(&content, &players),
        Some(PerkEffect::NightVision)
    );
    let alive = health.single().is_ok_and(|h| h.alive());
    if !has || !alive {
        nv.on = false;
        return;
    }
    if controls.night_vision && *screen == Screen::None {
        nv.on = !nv.on;
        sounds.write(PlaySfx::ui(if nv.on { Sfx::Radar } else { Sfx::UiMove }));
    }
}

fn spawn_tint(mut commands: Commands) {
    commands.spawn((
        MatchEntity,
        NightVisionTint,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.15, 1.0, 0.35, 0.16)),
        Visibility::Hidden,
        GlobalZIndex(-45),
    ));
}

fn tint(nv: Res<NightVision>, mut overlay: Query<&mut Visibility, With<NightVisionTint>>) {
    if let Ok(mut visibility) = overlay.single_mut() {
        let wanted = if nv.on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Every robot gets a hidden heat silhouette riding its body.
fn give_heat(
    mut commands: Commands,
    art: Res<Art>,
    robots: Query<&Rig, (Added<Rig>, With<PlayerRobot>)>,
) {
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

/// Glow the enemy silhouettes while a thermal user aims.
fn show_heat(
    content: Res<Content>,
    rules: Res<Rules>,
    view: Res<ViewState>,
    local: Query<&Player, With<LocalPlayer>>,
    robots: Query<(&Rig, &Player, &Health), With<PlayerRobot>>,
    mut glows: Query<(&ChildOf, &mut Visibility), With<ThermalGlow>>,
) {
    let me = local.single().ok();
    let thermal =
        matches!(local_perk(&content, &local), Some(PerkEffect::Thermal)) && view.ads > 0.6;
    for (parent, mut visibility) in &mut glows {
        let hot = thermal
            && robots
                .iter()
                .find(|(rig, ..)| rig.body == parent.parent())
                .is_some_and(|(_, player, health)| {
                    health.alive()
                        && me.is_some_and(|me| {
                            player.id != me.id && rules.pvp && player.team != me.team
                        })
                });
        let wanted = if hot {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}
