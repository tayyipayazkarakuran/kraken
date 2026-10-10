# Kraken

[Türkçe](README.tr.md)

Kraken is an automation assistant for the Seafight game client (Unity / IL2CPP) on Windows. A small agent runs inside the game, reads the game state, and carries out actions through the game's own systems. A control panel is drawn inside the game window, and it is also available as a separate window.

> **Disclaimer.** This is an independent hobby project with no connection to Bigpoint or Seafight. Automating the game may break its terms of service and can lead to account restrictions. You use it at your own risk.

## Features

| Area | What it does |
|---|---|
| NPC and monsters | Picks targets from your lists by priority, approaches them, and fires the chosen ammunition or harpoon |
| Collector | Collects the box types you choose, including small detours during a fight |
| Repair | Repairs below a threshold. While repairing it sails to the map corner opposite the ship that is attacking it |
| Death and connection | Respawns with the standard option, reconnects after short drops, and waits out long outages |
| Bonus maps | Plays the queued bonus maps one after another and spreads its fire across each wave |
| El Dorado | When the master skill is charged, it retreats to the nearest corner, jumps, and collects the chests |
| Work maps | Works on each listed map for a set time, then moves on. It teleports when it can, and otherwise sails from edge to edge |
| War helper | In PvP the player steers. Kraken selects targets, fires, and uses item rules |
| Items | Rules for when to use action items, by scene, condition, and interval |
| In-game interface | HUD, range ring, and target markers, plus the full panel on F9 |
| Multiple clients | One session per game client, with settings kept per account |

## Download

On the [Releases](../../releases) page, download `Kraken-<version>-win64.zip`, unzip it, and run `kraken.exe`. Everything is in that single file: the agent and the injector are embedded and unpacked to `%LOCALAPPDATA%\Kraken\bin` on first start.

1. Start `kraken.exe`. It can open the Seafight launcher for you (Settings → Connection).
2. Log in and press **Play**. Kraken attaches to the game client automatically.
3. Press **F9** in the game to open the panel. Set up your targets, then press **Start**.

Settings, logs, and the game data cache are stored in `%LOCALAPPDATA%\Kraken`.

## Building from source

You need Windows and the Rust toolchain. `rust-toolchain.toml` installs both the `x86_64` and `i686` MSVC targets.

```bash
cargo xtask dist
```

This command builds the 32-bit agent and injector, embeds them in the 64-bit app, and writes `dist/Kraken/kraken.exe`.

```bash
cargo xtask check
```

This runs formatting, clippy (`-D warnings`), and all the tests.

## Project layout

| Path | Contents |
|---|---|
| `crates/kraken-app` | Control panel (egui), session supervisor, game data (loca bundles), in-game UI frames |
| `crates/kraken-agent` | The DLL inside the game: IL2CPP bindings, message hooks, D3D11 overlay, and the decision engine (`src/engine`) |
| `crates/kraken-injector` | A small 32-bit injector |
| `crates/kraken-proto` | The app ↔ agent protocol: configuration, telemetry, and named-pipe framing |
| `xtask` | `dist`, `check`, and `reload` (hot-reloads the agent during development) |

The engine is plain Rust with no game calls in it. It takes in a `World` snapshot and returns `Action`s, so all of its behaviour (target choice, repair flight, bonus maps, work-map routing, and so on) is covered by unit tests.

## License

All rights reserved. The source is shared so it can be read and studied.
