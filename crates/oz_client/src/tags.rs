//! Name tags: teammates' names float over their heads, and an enemy's name
//! shows when your crosshair is on them and nothing is in the way.

use crate::{ClientSet, lobby::TEAM_COLORS, ui::Screen, view::WorldCamera};
use bevy::prelude::*;
use oz_sim::prelude::*;

const POOL: usize = 16;
const TEAM_RANGE: f32 = 70.0;
const ENEMY_RANGE: f32 = 90.0;

#[derive(Component)]
struct Tag(usize);

pub struct TagsPlugin;

impl Plugin for TagsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(MatchPhase::Active), spawn_tags).add_systems(Update, place_tags.in_set(ClientSet::Present).after(crate::spectate::Spectating).run_if(in_state(MatchPhase::Active)));
    }
}

fn spawn_tags(mut commands: Commands) {
    for i in 0..POOL {
        commands
            .spawn((MatchEntity, Tag(i), Node { position_type: PositionType::Absolute, width: Val::Px(220.0), margin: UiRect::left(Val::Px(-110.0)), justify_content: JustifyContent::Center, ..default() }, Visibility::Hidden, GlobalZIndex(-42)))
            .with_children(|tag| {
                tag.spawn((Text::new(""), TextFont { font_size: 13.0, ..default() }, TextColor(Color::WHITE), TextLayout::new_with_justify(JustifyText::Center)));
            });
    }
}

#[allow(clippy::too_many_arguments)]
fn place_tags(
    screen: Res<Screen>,
    rules: Res<Rules>,
    statics: Res<Statics>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    local: Query<&Player, With<LocalPlayer>>,
    others: Query<(&Player, &Transform, &PlayerInput, &Health), Without<LocalPlayer>>,
    mut tags: Query<(&Tag, &mut Node, &mut Visibility, &Children)>,
    mut texts: Query<(&mut Text, &mut TextColor)>,
    smokes: Query<(&Transform, &SmokeCloud)>,
) {
    let (Ok((camera, view)), Ok(me)) = (camera.single(), local.single()) else { return };
    let eye = view.translation();
    let forward = view.forward().as_vec3();
    let mut shown: Vec<(Vec3, String, Color)> = Vec::new();
    if matches!(*screen, Screen::None | Screen::Chat) {
        for (player, transform, input, health) in &others {
            if !health.alive() {
                continue;
            }
            let head = transform.translation + Vec3::Y * (stance::eye(input.stance) + 0.55);
            let to = head - eye;
            let distance = to.length();
            let friend = !rules.pvp || player.team == me.team;
            let aimed = to.normalize_or_zero().dot(forward) > 0.9985;
            let visible = || statics.raycast(eye, to / distance.max(1e-3), distance).is_none() && !smoke_between(&smokes, eye, head);
            if friend && distance < TEAM_RANGE || !friend && aimed && distance < ENEMY_RANGE && visible() {
                let color = if friend { TEAM_COLORS[(player.team as usize).min(1)] } else { Color::srgb(1.0, 0.35, 0.3) };
                shown.push((head, player.name.clone(), color));
            }
        }
    }
    for (tag, mut node, mut visibility, children) in &mut tags {
        let projected = shown.get(tag.0).and_then(|(at, name, color)| camera.world_to_ndc(view, *at).filter(|ndc| ndc.z > 0.0 && ndc.z < 1.0).map(|ndc| (ndc, name, color)));
        let Some((ndc, name, color)) = projected else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        *visibility = Visibility::Inherited;
        node.left = Val::Percent((ndc.x + 1.0) * 50.0);
        node.top = Val::Percent((1.0 - ndc.y) * 50.0);
        if let Some(&child) = children.first() {
            if let Ok((mut text, mut tint)) = texts.get_mut(child) {
                if text.0 != *name {
                    text.0 = name.clone();
                }
                tint.0 = *color;
            }
        }
    }
}
