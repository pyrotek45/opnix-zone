# Making a game from the parts

`games/iron_town/src/lib.rs` is the reference: a whole game in one file.

## 1. Describe your content

Return a `Catalog` from a function. Start from the shared kit and change what you need:

```rust
pub fn catalog(_: &Rules) -> Catalog {
    Catalog {
        weapons: oz_core::kit::standard_guns(),
        attachments: standard_attachments(),
        enemies: vec![/* EnemyDef { hp, speed, brain, model, .. } */],
        ..default()
    }
}
```

Models are built in code:

```rust
BlockModel::new()
    .cube(Paint::Steel, [0.0, 0.6, 0.0], [1.0, 0.6, 1.7])
    .pair(Shape::Cube, Paint::Dark, [0.6, 0.3, -0.5], [0.24, 0.6, 0.24], Some(0.0)) // animated legs
    .shard(Paint::Blue, [0.0, 1.2, 0.0], [0.3, 0.3, 0.3]).anim(Anim::Pulse)
```

## 2. Describe your rules

`Rules` picks the map recipe and the engine switches: `pvp`, `team_size`,
`score_limit`, `mining`, `objective_hp`, `day_seconds`, `respawn_seconds`,
starting stock and starter weapons.

## 3. Write a director

A director is a normal Bevy system in `SimSet::Logic`. It reads and writes
`MatchState` and calls engine helpers:

* `spawn_enemy(...)` - enemies from your catalog.
* `oz_sim::bots::spawn_bot(...)` - AI players that use the normal controller.
* `FxEvent::notice(...)`, `FxEvent::Sound { .. }` - feedback on every machine.
* `PlayerCommand::Mode(n)` - your own buttons (declare them in `Catalog::mode_actions`).

## 4. Register and run

```rust
app.insert_resource(Modes(vec![ModeDef { name: "MY MODE", blurb: "...", playable: true, rules, catalog }]))
   .add_systems(Update, my_director.in_set(SimSet::Logic));
```

```rust
App::new()
    .add_plugins((ClientPlugin { title: "MY GAME".into() }, SimPlugin, MyGamePlugin))
    .add_systems(Startup, launch_from_args)
    .run();
```

Menus, hosting, joining, HUD, stations, loadouts, bots and audio all work without further code.

## 5. Test it headlessly

Copy `games/iron_town/tests/versus.rs`: `MinimalPlugins + StatesPlugin + SimPlugin + YourPlugin`,
a fixed timestep, and assertions on `MatchState`.
