# Roadmap

## Planned modes (already listed in the menus)

| Mode | Built from | Still needed |
|------|-----------|--------------|
| **Outbreak** - round-based undead survival | wave director, stations as wall buys, perks | door / barrier purchases, mystery crate, new enemy set |
| **Domination / Search and Destroy** | Iron Town, teams, bots | capture zones, round logic, bot objectives |
| **Conquest** - large maps, many players | teams, bots, chunked terrain | bigger map layouts, vehicles, squad spawning, interest-managed snapshots |
| **Bastion** - tower defense | structures, turrets, wave director | fixed lanes, path following, tower upgrade trees |

## Known limitations

* **Buildings are solid blocks.** Towns have streets, alleys and cover but no interiors yet.
* **No lag compensation or shot prediction.** The host resolves shots from each
  client's reported aim, and a client sees its own shots after one round trip.
  Fine on LAN and good connections; competitive play over poor links will want both.
* **Movement is owner-authoritative.** It feels instant but trusts clients;
  there is no anti-cheat.
* **TCP transport, direct connections only.** Internet play needs a forwarded
  port or a mesh VPN; there is no relay, lobby browser or NAT punch-through.
* **Bots use simple steering,** not path-finding. They handle the town grid
  but can be slow around complex cover.
* **Snapshots are full-state.** They are compact, but very large matches will
  want delta compression and interest management.
* **Mining logistics are abstract.** Drills feed the shared stock directly;
  conveyors, refineries and transport are future work.

## Engine wishlist

More map layouts (mountains, woods, larger towns with enterable buildings),
weapon types (charge, burst, melee), per-enemy hit flashes, a scoreboard,
key rebinding, saved progress, and a newer Bevy release.
