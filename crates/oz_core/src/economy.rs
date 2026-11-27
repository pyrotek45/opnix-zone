//! Shared team stock and prices.

use serde::{Deserialize, Serialize};

/// Ore types. Indexes into [`Stock::ore`] and [`Cost::ore`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ore {
    Iron = 0,
    Copper = 1,
    Aether = 2,
}

impl Ore {
    pub const ALL: [Ore; 3] = [Ore::Iron, Ore::Copper, Ore::Aether];

    pub fn name(self) -> &'static str {
        ["IRON", "COPPER", "AETHER"][self as usize]
    }

    pub fn from_index(i: u8) -> Ore {
        Self::ALL[(i as usize).min(2)]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    pub credits: u32,
    pub ore: [u32; 3],
}

impl Cost {
    pub const FREE: Cost = Cost {
        credits: 0,
        ore: [0; 3],
    };

    pub const fn credits(credits: u32) -> Self {
        Self {
            credits,
            ore: [0; 3],
        }
    }

    pub const fn new(credits: u32, iron: u32, copper: u32, aether: u32) -> Self {
        Self {
            credits,
            ore: [iron, copper, aether],
        }
    }

    /// Price after `level` previous purchases: +60% per level.
    pub fn scaled(self, level: u32) -> Self {
        let m = |v: u32| (v as f32 * (1.0 + 0.6 * level as f32)).round() as u32;
        Self {
            credits: m(self.credits),
            ore: self.ore.map(m),
        }
    }

    /// Short label such as `$120 · 20 IRON`.
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.credits > 0 {
            parts.push(format!("${}", self.credits));
        }
        for ore in Ore::ALL {
            let n = self.ore[ore as usize];
            if n > 0 {
                parts.push(format!("{n} {}", ore.name()));
            }
        }
        if parts.is_empty() {
            "FREE".into()
        } else {
            parts.join(" + ")
        }
    }
}

/// What the team owns. Shared between all players in co-op modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stock {
    pub credits: u32,
    pub ore: [u32; 3],
}

impl Stock {
    pub fn can_afford(&self, cost: &Cost) -> bool {
        self.credits >= cost.credits && (0..3).all(|i| self.ore[i] >= cost.ore[i])
    }

    /// Deducts `cost` if affordable. Returns whether the purchase happened.
    pub fn pay(&mut self, cost: &Cost) -> bool {
        if !self.can_afford(cost) {
            return false;
        }
        self.credits -= cost.credits;
        for i in 0..3 {
            self.ore[i] -= cost.ore[i];
        }
        true
    }

    pub fn add_ore(&mut self, ore: Ore, amount: u32) {
        self.ore[ore as usize] = self.ore[ore as usize].saturating_add(amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pay_is_all_or_nothing() {
        let mut stock = Stock {
            credits: 100,
            ore: [5, 0, 0],
        };
        assert!(!stock.pay(&Cost::new(50, 6, 0, 0)));
        assert_eq!(stock.credits, 100);
        assert!(stock.pay(&Cost::new(50, 5, 0, 0)));
        assert_eq!(
            stock,
            Stock {
                credits: 50,
                ore: [0; 3]
            }
        );
        assert_eq!(Cost::new(100, 10, 0, 0).scaled(1).credits, 160);
    }
}
