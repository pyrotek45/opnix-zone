# Architecture

```
crates/
  oz_core     pure Rust, no Bevy   data + algorithms: worldgen, block models, arsenal, content, synth
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
  `PlayerCommand`s for shared-state changes.
* The host broadcasts quantised snapshots (`EntState`, 15 bytes each).
  Fast entities (players, enemies, projectiles) go in every snapshot; slow
  ones (structures, ore) only in periodic full snapshots or when the set changes.
* One-shot effects travel as `FxEvent` batches, so every machine replays the
  same tracers, explosions and sounds.
* Clients rebuild the same components the host has (`Enemy`, `Structure`...),
  which is why the presentation code is identical on both sides.
* The map is never sent: host and clients generate it from the same `WorldRecipe`.

`oz_net` hides the wire. Swapping TCP for UDP, a relay, or a platform socket
layer does not touch gameplay code.

## Assets from code

* **Models** are `BlockModel`s: lists of painted primitives with optional
  animation tags. `oz_client::art` bakes each into at most four meshes.
* **Maps** come from `oz_core::worldgen` (`Layout::Archipelago`, `Layout::Town`).
  A new generator only has to fill in the `World` struct.
* **Sounds** come from `oz_core::synth`, rendered to WAV in memory at startup.
* **Sky** is a generated cubemap; day/night is a blend driven by `MatchState::daylight`.
