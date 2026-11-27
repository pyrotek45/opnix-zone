//! Getting back into a match after the connection drops.
//!
//! When the host vanishes (not when you choose to leave), the simulation
//! records a [`LostHost`]. Back on the main menu this module retries the
//! same address a few times, a couple of seconds apart, with the same
//! reconnect token, so the host can hand back your seat, team, score and
//! weapons. If every attempt fails a RECONNECT button stays on the menu.

use crate::{
    ClientSet,
    ui::{Act, ActEvent, UiDirty},
};
use bevy::prelude::*;
use oz_sim::{
    net::{self, JoinStatus, Joining, LostHost},
    prelude::*,
};

const ATTEMPTS: u8 = 4;
const GAP: f32 = 2.5;

#[derive(Resource, Default)]
pub struct Reconnect {
    /// Attempts made since the connection dropped.
    attempts: u8,
    wait: f32,
    /// Something to reconnect to.
    lost: bool,
}

impl Reconnect {
    /// A line for the main menu while retrying.
    pub fn status(&self) -> Option<String> {
        (self.lost && self.attempts > 0 && self.attempts <= ATTEMPTS && !self.gave_up()).then(|| format!("LOST THE HOST - RECONNECTING ({}/{ATTEMPTS})...", self.attempts))
    }

    pub fn gave_up(&self) -> bool {
        self.lost && self.attempts >= ATTEMPTS && self.wait <= 0.0
    }
}

pub struct ReconnectPlugin;

impl Plugin for ReconnectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Reconnect>()
            .add_systems(OnEnter(MatchPhase::Active), |mut reconnect: ResMut<Reconnect>, mut commands: Commands| {
                *reconnect = Reconnect::default();
                commands.remove_resource::<LostHost>();
            })
            .add_systems(Update, (retry, manual).in_set(ClientSet::Input).run_if(not(in_state(MatchPhase::Active))));
    }
}

fn retry(time: Res<Time<Real>>, lost: Option<Res<LostHost>>, joining: Option<Res<Joining>>, join: Res<JoinStatus>, mut reconnect: ResMut<Reconnect>, mut dirty: ResMut<UiDirty>, mut commands: Commands) {
    let Some(lost) = lost else {
        reconnect.lost = false;
        return;
    };
    if !reconnect.lost {
        *reconnect = Reconnect { lost: true, attempts: 0, wait: 1.0 };
    }
    // One attempt at a time; wait for the last to finish.
    if joining.is_some() || matches!(*join, JoinStatus::Connecting | JoinStatus::Finding(_)) {
        return;
    }
    let was_waiting = reconnect.wait > 0.0;
    reconnect.wait -= time.delta_secs();
    if reconnect.wait > 0.0 {
        return;
    }
    if reconnect.attempts >= ATTEMPTS {
        if was_waiting {
            dirty.0 = true;
        }
        return;
    }
    reconnect.attempts += 1;
    reconnect.wait = GAP;
    // A code goes back through the board, so both routers are punched again.
    match &lost.code {
        Some(code) => net::join_code(&mut commands, code, lost.name.clone(), lost.token),
        None => net::join_as(&mut commands, &lost.address, lost.name.clone(), lost.token),
    }
    dirty.0 = true;
}

fn manual(mut events: EventReader<ActEvent>, lost: Option<Res<LostHost>>, mut reconnect: ResMut<Reconnect>, mut dirty: ResMut<UiDirty>) {
    for ActEvent(act) in events.read() {
        if *act == Act::Reconnect && lost.is_some() {
            reconnect.attempts = ATTEMPTS - 1;
            reconnect.wait = 0.0;
            dirty.0 = true;
        }
    }
}
