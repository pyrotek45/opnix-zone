# Architecture

```
crates/
  oz_core     pure Rust, no Bevy   data + algorithms: worldgen, collision, block models, modular arsenal, content, synth
  oz_net      pure Rust, no Bevy   host/client message transport (framed TCP)
  oz_sim      Bevy ECS, headless   the authoritative simulation + replication
  oz_client   Bevy render/UI       everything seen, heard and touched
games/
  ember_atoll  content + siege director (+ dedicated server binary)
  iron_town    content + deathmatch director
```

Dependencies only point downward: `games -> oz_client -> oz_sim -> oz_core / oz_net`.

## The three rules

1. **The simulation never renders, the client never decides.** `oz_sim` runs
   identically in single-player, on a host, on a dedicated server and in tests.
   `oz_client` only reads simulation state and emits input.
2. **Everything a player changes is a command.** Purchases, builds, upgrades
   and classes are `PlayerCommand`s validated in `oz_sim::orders`. Offline and
   online play share that one code path.
3. **Content is data.** Weapons, enemies, structures, upgrades, stations, perks
   and gear live in a `Catalog` supplied by the game. Engine crates never name
   a specific gun or monster.

## A frame

```
ClientSet::Input    read devices -> Controls -> PlayerInput / CommandEvent, menus
SimSet::Receive     network in (host: inputs, client: snapshots)
  movement          owner-controlled players and bots
SimSet::Orders      authority: validate and apply commands
SimSet::Logic       authority: director, bots, weapons, AI, structures, projectiles
SimSet::Resolve     authority: apply hits, kills, rewards
SimSet::Send        network out (host: snapshots + effects, client: input)
ClientSet::Present  attach/animate visuals, effects, HUD, camera
```

## Networking

Listen-server model: the host owns the simulation and also plays.

* Clients send `PlayerInput` 30 times a second plus their own position
  (movement is owner-authoritative so it always feels instant), and
  `PlayerCommand`s for shared-state changes. Buttons pressed between sends are
  latched into the next one, so a one-frame tap still arrives.
* The host broadcasts quantised snapshots (`EntState`, 17 bytes each),
  stamped with the host clock. Fast entities go in every snapshot; slow ones
  (structures, ore) only in periodic full snapshots or when the set changes.
* Clients keep a `ServerClock` and play every remote entity back
  `INTERPOLATION` (0.1 s) behind it from a small sample buffer (`Remote`).
* **Lag compensation** (`oz_sim::lagcomp`): the host records every player's
  true pose each frame. Inputs carry the client's playback time; when a remote
  player fires, `weapons` tests the shot against everyone rewound to that
  time (capped at 0.35 s). Poses include the lean offset. While guests are
  connected, co-op enemies' hit spheres are recorded too and rewound the
  same way.
* **Prediction** (`oz_client::predict`): on a client, the trigger, fire rate
  and bursts run locally so your shots, recoil and ammo count are instant. The
  host's shot counter confirms them; the host's echo of your own shots is
  dropped. Rockets and thrown gear spawn a local `Ghost` projectile; the
  host's replicated copy (tagged with its owner) stays hidden for you.
* **Reconnecting**: `C2S::Hello` carries a per-install token. The host keeps a
  departed player's seat (id, team, score, class, arsenal) for ten minutes and
  restores it on a hello with the same token, kicking any stale connection
  that still holds it. Clients record a `LostHost` when the host vanishes and
  `oz_client::reconnect` retries it.
* One-shot effects travel as `FxEvent` batches, so every machine replays the
  same tracers, explosions and sounds.
* Pings run once a second; silent connections are dropped (6 s on a client,
  10 s on the host).
* `oz_net::lan` adds LAN discovery (UDP broadcast on 27651) and UPnP port
  forwarding on a background thread.
* The map is never sent: host and clients generate it from the same `WorldRecipe`.

`oz_net` hides the wire. Swapping TCP for UDP, a relay, or a platform socket
layer does not touch gameplay code.

## Collision and movement

`oz_core::collide::StaticWorld` holds every wall, floor, stair step, crate and
prop as an axis-aligned box in a 4 m grid. It answers three questions: is a
body blocked here, what floor is under this point (so stairs and upper
storeys work), and where does this ray first hit (bullets, line of sight,
grenades). Player-built structures are separate, dynamic `Solid`s.

Buildings come from `worldgen` as a list of `Block`s - walls cut around doors
and windows, floor slabs with stairwells, stairs, railings, furniture, roofs
and decorative trims. The same list drives rendering and collision.

## Bots

`oz_sim::nav` builds a layered half-metre grid from the map and collision at
match start: each column holds one node per surface you can stand on (the
ground, every stair step, upper floors, roofs), and neighbours connect when
the height change is walkable, so stairs join floors. A* runs over the nodes
and the result is smoothed into straight legs that never jump floors. Bots
(`oz_sim::bots`) only ever write `PlayerInput`, exactly like a human: they
travel by path, engage at ranges that suit their weapon, crouch or go prone
for long shots, lean out from cover when an enemy is only visible that way,
are stunned by flashbangs and blind behind smoke, and have reaction time and
aim error scaled by difficulty.
Searches are limited per frame so a full lobby never spikes.

## Versus

`oz_sim::versus` is switched on by `Rules::pvp`. It runs the lobby (ready,
start countdown, switch team, team size), keeps teams full with bots,
counts kills toward the score limit, and returns everyone to the lobby for a
rematch. `MatchState::roster` carries names, teams, ready flags and K/D to
every machine; it feeds the lobby, the scoreboard and remote players' names.

## Players as robots

Every player and bot is drawn as a jointed robot with a CRT head
(`oz_client::avatar`). A `Pose` (speed, stance, look pitch, aiming,
reloading, sprinting, dead) drives a procedural rig: run cycle, crouch,
prone, aiming up and down, reload tilt, falling over, and the face on the
screen. Stance, aim and reload state travel in each snapshot's `flags` byte,
and look pitch in its `pitch` byte, so everyone sees what everyone is doing.

## Assets from code

* **Models** are `BlockModel`s: lists of painted primitives with optional
  animation tags. `oz_client::art` bakes each into at most four meshes.
  Guns are assembled from their frame and fitted parts by `arsenal::assemble`.
* **Maps** come from `oz_core::worldgen` (`Layout::Archipelago`, `Layout::Town`).
  A new generator only has to fill in the `World` struct.
* **Sounds** come from `oz_core::synth`, rendered to WAV in memory at startup.
* **Sky** is a generated cubemap; day/night is a blend driven by `MatchState::daylight`.
