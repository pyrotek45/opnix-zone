# Roadmap

## Planned modes (already listed in the menus)

| Mode | Built from | Still needed |
|------|-----------|--------------|
| **Outbreak** - round-based undead survival | wave director, stations as wall buys, perks | door / barrier purchases, mystery crate, new enemy set |
| **Domination / Search and Destroy** | Iron Town, teams, bots | capture zones, round logic, bot objectives |
| **Conquest** - large maps, many players | teams, bots, chunked terrain | bigger map layouts, vehicles, squad spawning, interest-managed snapshots |
| **Bastion** - tower defense | structures, turrets, wave director | fixed lanes, path following, tower upgrade trees |

## Known limitations

* **Remote players' attachments** are shown with each gun's factory parts on
  other clients (the host sees bots' real parts). Stats are always correct.
* **Thrown gear is predicted, not reconciled.** Your own grenades and rockets
  fly at once on your screen; the host's copy decides where they land, and
  the two can differ slightly when spread or a moving target is involved.
* **Movement is owner-authoritative.** It feels instant but trusts clients;
  there is no anti-cheat.
* **TCP transport, direct connections.** The host tries UPnP; routers without
  it need a forwarded port or a mesh VPN. There is no relay, internet server
  browser or NAT punch-through.
* **Gamepad buttons are fixed;** keyboard and mouse can be rebound.
* **Snapshots are full-state.** They are compact, but very large matches will
  want delta compression and interest management.
* **Mining logistics are abstract.** Drills feed the shared stock directly;
  conveyors, refineries and transport are future work.

## Engine wishlist

More map layouts (mountains, woods, larger towns with enterable buildings),
weapon types (charge, melee), per-enemy hit flashes, bots that use gear,
gamepad rebinding, saved progress, and a newer Bevy release.
