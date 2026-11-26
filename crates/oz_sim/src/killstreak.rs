//! Killstreaks (versus modes).
//!
//! Kills without dying earn rewards; the player calls one in when they want
//! (a key / button that sends [`PlayerCommand::UseStreak`]):
//!
//! | Kills | Reward | What it does |
//! |------:|--------|--------------|
//! | 3 | Recon UAV | 30 s of every enemy showing on your team's radar |
//! | 5 | Airstrike | a jet drops a line of bombs where you are aiming |
//! | 7 | Attack drone | hovers over you for 30 s and shoots enemies it can see |
//!
//! Streaks reset on death; earned rewards wait until used. Kills made by a
//! reward count toward the next one.

use crate::{
    combat::{SmokeCloud, blast_players, smoke_between},
    net::NetIds,
    player::{Health, Player, PlayerInput, damage_player, eye_point},
    session::{Content, Map, MatchEntity, Statics},
    sim_dt,
    state::{CommandEvent, FxEvent, MatchState, Phase, PlayerCommand, Rules},
};
use bevy::prelude::*;
use oz_core::synth::Sfx;

pub mod reward {
    pub const UAV: u8 = 1;
    pub const AIRSTRIKE: u8 = 2;
    pub const DRONE: u8 = 4;
}

pub struct StreakDef {
    pub kills: u16,
    pub bit: u8,
    pub name: &'static str,
}

pub const STREAKS: [StreakDef; 3] = [
    StreakDef { kills: 3, bit: reward::UAV, name: "RECON UAV" },
    StreakDef { kills: 5, bit: reward::AIRSTRIKE, name: "AIRSTRIKE" },
    StreakDef { kills: 7, bit: reward::DRONE, name: "ATTACK DRONE" },
];

pub fn name(bit: u8) -> &'static str {
    STREAKS.iter().find(|s| s.bit == bit).map_or("REWARD", |s| s.name)
}

/// The reward the next call-in will use: the cheapest one earned.
pub fn next(rewards: u8) -> Option<&'static StreakDef> {
    STREAKS.iter().find(|s| rewards & s.bit != 0)
}

const UAV_SECONDS: f32 = 30.0;
const STRIKE_DELAY: f32 = 2.6;
const BOMBS: u8 = 7;
const BOMB_SPACING: f32 = 3.6;
const BOMB_DAMAGE: f32 = 170.0;
const BOMB_RADIUS: f32 = 6.0;
const DRONE_SECONDS: f32 = 30.0;
const DRONE_RANGE: f32 = 45.0;
const DRONE_DAMAGE: f32 = 13.0;
const DRONE_RATE: f32 = 0.32;
/// A sentinel shooter id for drone tracers, so the owner's client does not
/// mistake them for its own predicted shots.
pub const DRONE_SHOOTER: u8 = 254;

#[derive(Component, Clone, Copy, Debug)]
pub struct Airstrike {
    pub at: Vec3,
    pub dir: Vec3,
    pub owner: u8,
    pub team: u8,
    pub timer: f32,
    pub flown: bool,
    pub dropped: u8,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct Drone {
    pub owner: u8,
    pub team: u8,
    pub life: f32,
    pub cooldown: f32,
    /// Where it circles, around the owner.
    pub orbit: f32,
}

/// Authority: turns call-in commands into rewards.
#[allow(clippy::too_many_arguments)]
pub fn use_streaks(
    mut orders: EventReader<CommandEvent>,
    rules: Res<Rules>,
    map: Res<Map>,
    statics: Res<Statics>,
    mut state: ResMut<MatchState>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut players: Query<(&mut Player, &Transform, &PlayerInput, &Health)>,
    mut fx: EventWriter<FxEvent>,
) {
    for order in orders.read() {
        if order.command != PlayerCommand::UseStreak {
            continue;
        }
        let Some((mut player, transform, input, health)) = players.iter_mut().find(|(p, ..)| p.id == order.player) else { continue };
        let Some(def) = next(player.rewards).filter(|_| rules.pvp && health.alive() && state.phase == Phase::Live) else {
            fx.write(FxEvent::Cue { to: order.player, sfx: Sfx::UiDeny });
            continue;
        };
        player.rewards &= !def.bit;
        let team = player.team;
        match def.bit {
            reward::UAV => state.uav[(team as usize).min(1)] = UAV_SECONDS,
            reward::AIRSTRIKE => {
                // Bombs fall along the line you are looking down, centred where you aim.
                let eye = eye_point(&statics, transform.translation, input);
                let dir = input.aim_dir();
                let mut reach = map.raycast(eye, dir, 150.0);
                if let Some((d, _)) = statics.raycast(eye, dir, reach) {
                    reach = reach.min(d);
                }
                let at = eye + dir * reach;
                let flat = Vec3::new(dir.x, 0.0, dir.z).normalize_or(Vec3::NEG_Z);
                commands.spawn((MatchEntity, Airstrike { at, dir: flat, owner: player.id, team, timer: STRIKE_DELAY, flown: false, dropped: 0 }));
            }
            _ => {
                let at = transform.translation + Vec3::Y * 4.5;
                commands.spawn((MatchEntity, ids.next(), Drone { owner: player.id, team, life: DRONE_SECONDS, cooldown: 1.0, orbit: 0.0 }, Transform::from_translation(at)));
            }
        }
        fx.write(FxEvent::StreakUsed { name: player.name.clone(), team, reward: def.bit });
    }
}

/// Authority: UAV timers, falling bombs and hunting drones.
#[allow(clippy::too_many_arguments)]
pub fn run_streaks(
    time: Res<Time>,
    map: Res<Map>,
    statics: Res<Statics>,
    content: Res<Content>,
    grid: Res<crate::combat::Grid>,
    mut state: ResMut<MatchState>,
    mut commands: Commands,
    mut strikes: Query<(Entity, &mut Airstrike)>,
    mut drones: Query<(Entity, &mut Drone, &mut Transform), Without<Player>>,
    mut players: Query<(&Transform, &mut Health, &mut Player), (Without<crate::combat::Projectile>, Without<crate::structures::Structure>, Without<Drone>)>,
    smokes: Query<(&Transform, &SmokeCloud), (Without<Player>, Without<Drone>)>,
    mut hits: EventWriter<crate::combat::Hit>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    for uav in &mut state.uav {
        *uav = (*uav - dt).max(0.0);
    }
    let name_of = |state: &MatchState, id: u8| state.roster.iter().find(|r| r.id == id).map_or(String::new(), |r| r.name.clone());

    for (entity, mut strike) in &mut strikes {
        strike.timer -= dt;
        if !strike.flown && strike.timer <= 0.7 {
            strike.flown = true;
            let high = strike.at + Vec3::Y * 38.0;
            fx.write(FxEvent::Flyby { from: high - strike.dir * 160.0, to: high + strike.dir * 160.0 });
        }
        // One bomb every 0.12 s once the timer runs out.
        while strike.timer <= -(strike.dropped as f32) * 0.12 && strike.dropped < BOMBS {
            let along = (strike.dropped as f32 - (BOMBS - 1) as f32 * 0.5) * BOMB_SPACING;
            let p = strike.at + strike.dir * along;
            let ground = map.height(p.x, p.z).max(statics.support(p.x, p.z, p.y + 0.5));
            let at = Vec3::new(p.x, ground.max(p.y - 3.0) + 0.4, p.z);
            fx.write(FxEvent::Explosion { at, radius: BOMB_RADIUS });
            crate::combat::splash(&grid, &mut hits, at, BOMB_RADIUS, BOMB_DAMAGE, Some(strike.owner), None);
            blast_players(&content, &mut state, &mut fx, &mut players, at, BOMB_RADIUS, BOMB_DAMAGE, strike.team, strike.owner, None);
            strike.dropped += 1;
        }
        if strike.dropped >= BOMBS {
            commands.entity(entity).despawn();
        }
    }

    for (entity, mut drone, mut transform) in &mut drones {
        drone.life -= dt;
        let owner = players.iter().find(|(_, _, p)| p.id == drone.owner).map(|(t, h, _)| (t.translation, h.alive()));
        if drone.life <= 0.0 || owner.is_none() || state.phase != Phase::Live {
            fx.write(FxEvent::Explosion { at: transform.translation, radius: 1.5 });
            commands.entity(entity).despawn();
            continue;
        }
        // Circle above the owner (or where they fell).
        drone.orbit += dt * 0.9;
        let (owner_at, _) = owner.unwrap_or((transform.translation, false));
        let wanted = owner_at + Vec3::new(drone.orbit.cos() * 2.5, 4.5, drone.orbit.sin() * 2.5);
        transform.translation = transform.translation.lerp(wanted, 1.0 - (-dt * 2.5).exp());
        let at = transform.translation;

        drone.cooldown -= dt;
        // Nearest enemy it can actually see.
        let target = players
            .iter()
            .filter(|(_, h, p)| p.team != drone.team && h.alive() && p.protect <= 0.0)
            .map(|(t, _, p)| (p.id, t.translation + Vec3::Y * 1.1))
            .filter(|(_, chest)| {
                let span = *chest - at;
                let d = span.length();
                d < DRONE_RANGE && statics.raycast(at, span / d.max(1e-3), d).is_none() && map.raycast(at, span / d.max(1e-3), d) >= d - 0.3 && !smoke_between(smokes, at, *chest)
            })
            .min_by(|a, b| a.1.distance_squared(at).total_cmp(&b.1.distance_squared(at)));
        let Some((victim, chest)) = target else { continue };
        transform.look_at(Vec3::new(chest.x, at.y, chest.z), Vec3::Y);
        if drone.cooldown > 0.0 {
            continue;
        }
        drone.cooldown = DRONE_RATE;
        fx.write(FxEvent::Shot { by: DRONE_SHOOTER, sound: 1, from: at - Vec3::Y * 0.3, to: chest, beam: false, tracer: oz_core::model::Paint::Red, quiet: false, local: false });
        let name = name_of(&state, drone.owner);
        if let Some((_, mut health, mut player)) = players.iter_mut().find(|(_, _, p)| p.id == victim) {
            damage_player(&mut health, &mut player, DRONE_DAMAGE, Some((&name, drone.team, drone.owner)), &mut state, &mut fx);
        }
    }
}

/// Authority: bots call in what they earn - air support while they are in a firefight.
pub fn bots_call_in(state: Res<MatchState>, bots: Query<(&Player, &PlayerInput, &Health), With<crate::bots::Bot>>, mut orders: EventWriter<CommandEvent>) {
    if state.phase != Phase::Live {
        return;
    }
    for (player, input, health) in &bots {
        let Some(def) = next(player.rewards) else { continue };
        let fighting = input.held(crate::player::buttons::FIRE);
        if health.alive() && (def.bit != reward::AIRSTRIKE || fighting) {
            orders.write(CommandEvent { player: player.id, command: PlayerCommand::UseStreak });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cheapest_reward_goes_first() {
        assert_eq!(next(0).map(|s| s.bit), None);
        assert_eq!(next(reward::DRONE | reward::AIRSTRIKE).map(|s| s.bit), Some(reward::AIRSTRIKE));
        assert_eq!(name(reward::DRONE), "ATTACK DRONE");
    }
}
