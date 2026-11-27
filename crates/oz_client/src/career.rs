//! Career: levels, unlocks and rank, kept on this machine.
//!
//! * **Experience** comes from kills (more for headshots), match wins and clearing a
//!   terrorist hunt. Levels unlock guns: everyone starts with the basics, the rest
//!   open up level by level.
//! * **Weapon mastery**: kills with a gun unlock its attachments, slot by slot - the
//!   first option in each slot is free, every next one needs more kills with that gun.
//! * **Rank**: ranked matches move rank points up on a win and down on a loss, through
//!   bronze, silver, gold, platinum and diamond.
//!
//! The armory shows what is locked and what it takes; classes only ever use what is
//! unlocked. Saved next to the settings as `career.txt`.

use crate::{ClientSet, fx::PlaySfx, hud::Notices};
use bevy::prelude::*;
use oz_core::arsenal::{SLOT_COUNT, WeaponDef, options};
use oz_sim::prelude::*;
use std::fs;

/// Experience for each thing you do.
pub const XP_KILL: u32 = 100;
pub const XP_HEADSHOT: u32 = 50;
pub const XP_WIN: u32 = 500;
pub const XP_MATCH: u32 = 150;
/// Kills with a gun for each further attachment in a slot.
pub const KILLS_PER_PART: u32 = 12;
pub const DIVISIONS: [&str; 5] = ["BRONZE", "SILVER", "GOLD", "PLATINUM", "DIAMOND"];

#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct Career {
    pub xp: u32,
    /// Kills with each catalog weapon.
    pub kills: Vec<u32>,
    pub wins: u32,
    pub matches: u32,
    /// Ranked points (0 = bottom of bronze).
    pub rank: u32,
}

/// Total experience needed to reach `level` (level 1 is free).
pub fn xp_for(level: u32) -> u32 {
    let l = level.saturating_sub(1);
    250 * l * (l + 3) / 2
}

impl Career {
    pub fn level(&self) -> u32 {
        (1..=60).take_while(|l| xp_for(*l) <= self.xp).last().unwrap_or(1)
    }

    /// 0..1 through the current level.
    pub fn progress(&self) -> f32 {
        let level = self.level();
        let (from, to) = (xp_for(level), xp_for(level + 1));
        (self.xp - from) as f32 / (to - from).max(1) as f32
    }

    pub fn kills_with(&self, weapon: u8) -> u32 {
        self.kills.get(weapon as usize).copied().unwrap_or(0)
    }

    pub fn gun_unlocked(&self, weapon: u8) -> bool {
        self.level() >= gun_level(weapon)
    }

    /// Is this attachment (an index in the slot's option list) unlocked on this gun?
    pub fn part_unlocked(&self, def: &WeaponDef, content: &Content, weapon: u8, slot: usize, variant: u8) -> bool {
        let rank = options(def, &content.attachments, slot).iter().position(|(i, ..)| *i == variant as usize).unwrap_or(0) as u32;
        variant == def.defaults[slot] || self.kills_with(weapon) >= rank * KILLS_PER_PART
    }

    /// Kills needed with `weapon` for the option at position `rank` in a slot.
    pub fn part_kills(rank: usize) -> u32 {
        rank as u32 * KILLS_PER_PART
    }

    /// Division name and points into it.
    pub fn division(&self) -> (&'static str, u32) {
        let tier = (self.rank / 400).min(DIVISIONS.len() as u32 - 1);
        (DIVISIONS[tier as usize], self.rank - tier * 400)
    }

    /// Replaces anything locked in a class with what this career can use.
    pub fn enforce(&self, loadout: &mut Loadout, content: &Content) {
        let guns: Vec<u8> = (0..content.weapons.len() as u8).filter(|i| content.weapons[*i as usize].frame.is_some() && self.gun_unlocked(*i)).collect();
        let fallback = |taken: u8| guns.iter().copied().find(|g| *g != taken).unwrap_or(taken);
        if !self.gun_unlocked(loadout.primary) {
            loadout.primary = fallback(loadout.secondary);
        }
        if !self.gun_unlocked(loadout.secondary) || loadout.secondary == loadout.primary {
            loadout.secondary = fallback(loadout.primary);
        }
        let mut parts = loadout.parts.unwrap_or_default();
        for (side, weapon) in [loadout.primary, loadout.secondary].into_iter().enumerate() {
            let Some(def) = content.weapons.get(weapon as usize) else { continue };
            for slot in 0..SLOT_COUNT {
                if !self.part_unlocked(def, content, weapon, slot, parts[side][slot]) {
                    parts[side][slot] = def.defaults[slot];
                }
            }
        }
        loadout.parts = Some(parts);
    }

    fn path() -> Option<std::path::PathBuf> {
        crate::settings::config_dir().map(|dir| dir.join("career.txt"))
    }

    pub fn load() -> Self {
        let mut career = Career::default();
        let Some(text) = Self::path().and_then(|p| fs::read_to_string(p).ok()) else { return career };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match key {
                "xp" => career.xp = value.parse().unwrap_or(0),
                "wins" => career.wins = value.parse().unwrap_or(0),
                "matches" => career.matches = value.parse().unwrap_or(0),
                "rank" => career.rank = value.parse().unwrap_or(0),
                "kills" => career.kills = value.split(',').filter_map(|n| n.trim().parse().ok()).collect(),
                _ => {}
            }
        }
        career
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        let kills = self.kills.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let text = format!("xp={}\nwins={}\nmatches={}\nrank={}\nkills={kills}\n", self.xp, self.wins, self.matches, self.rank);
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(path, text);
    }
}

/// The level that unlocks a catalog weapon. The four basics are there from the start;
/// the rest open one every couple of levels, in catalog order.
pub fn gun_level(weapon: u8) -> u32 {
    use oz_core::kit::gun;
    match weapon as usize {
        gun::PISTOL | gun::SMG | gun::SHOTGUN | gun::CARBINE => 1,
        w => {
            let basics_before = [gun::PISTOL, gun::SMG, gun::SHOTGUN, gun::CARBINE].iter().filter(|b| **b < w).count();
            2 + 2 * (w - basics_before) as u32
        }
    }
}

pub struct CareerPlugin;

impl Plugin for CareerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Career::load()).add_systems(Update, earn.in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

/// Watches the match for what earns experience, mastery and rank.
#[allow(clippy::too_many_arguments)]
fn earn(
    mut career: ResMut<Career>,
    mut events: EventReader<FxEvent>,
    content: Res<Content>,
    rules: Res<Rules>,
    state: Res<MatchState>,
    mut notices: ResMut<Notices>,
    mut sounds: EventWriter<PlaySfx>,
    players: Query<(&Player, &PlayerInput), With<LocalPlayer>>,
    mut last_phase: Local<Option<Phase>>,
) {
    let Ok((me, input)) = players.single() else { return };
    let before = career.clone();
    for event in events.read() {
        if let FxEvent::Killed { victim, by, head, .. } = event
            && *by == me.id
            && *victim != me.id
        {
            career.xp += XP_KILL + if *head { XP_HEADSHOT } else { 0 };
            let weapon = input.weapon as usize;
            if career.kills.len() < content.weapons.len() {
                career.kills.resize(content.weapons.len(), 0);
            }
            if let Some(count) = career.kills.get_mut(weapon) {
                *count += 1;
            }
        }
    }
    // The end of a match.
    let phase = state.phase;
    if *last_phase == Some(Phase::Live) && phase == Phase::Won && rules.pvp {
        let won = state.winner == me.team;
        career.matches += 1;
        career.xp += XP_MATCH + if won { XP_WIN } else { 0 };
        if won {
            career.wins += 1;
        }
        if rules.ranked {
            career.rank = if won { career.rank + 25 } else { career.rank.saturating_sub(18) };
            let (division, points) = career.division();
            notices.push(format!("RANK  {division}  {points} / 400  ({}{})", if won { "+" } else { "-" }, if won { 25 } else { 18 }));
        }
    }
    *last_phase = Some(phase);
    if *career == before {
        return;
    }
    // Tell the player what just opened up.
    if career.level() > before.level() {
        notices.push(format!("LEVEL UP  -  LEVEL {}", career.level()));
        sounds.write(PlaySfx::ui(Sfx::Medal));
        for (index, def) in content.weapons.iter().enumerate().filter(|(_, w)| w.frame.is_some()) {
            if career.gun_unlocked(index as u8) && !before.gun_unlocked(index as u8) {
                notices.push(format!("NEW WEAPON UNLOCKED: {}", def.name));
            }
        }
    }
    let weapon = input.weapon;
    if let Some(def) = content.weapons.get(weapon as usize) {
        for (slot, name) in oz_core::arsenal::SLOTS.iter().enumerate() {
            for (rank, (_, part, ..)) in options(def, &content.attachments, slot).iter().enumerate() {
                let need = Career::part_kills(rank);
                if need > 0 && career.kills_with(weapon) >= need && before.kills_with(weapon) < need {
                    notices.push(format!("{}: {name} - {part} UNLOCKED", def.name));
                }
            }
        }
    }
    career.save();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_climb_and_unlock_guns_in_order() {
        let mut career = Career::default();
        assert_eq!(career.level(), 1);
        assert!(career.gun_unlocked(oz_core::kit::gun::CARBINE as u8));
        assert!(!career.gun_unlocked(oz_core::kit::gun::SNIPER as u8));
        career.xp = xp_for(12);
        assert_eq!(career.level(), 12);
        assert!(career.gun_unlocked(oz_core::kit::gun::SNIPER as u8));
        assert!(xp_for(3) > xp_for(2) && xp_for(2) > 0);
        // The last gun in the catalog still unlocks below the level cap.
        let last = oz_core::kit::standard_guns().len() as u8 - 1;
        assert!(gun_level(last) <= 50 && gun_level(last) > gun_level(oz_core::kit::gun::ROTARY as u8));
    }
}
