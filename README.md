# Claude Deck

Claude Deck is a Windows desktop app for running many Claude Code sessions, grouped by project.
A background helper (`deck-host`) keeps the sessions alive when the window closes and puts idle ones to sleep.
The window (Tauri + Vue) shows the session tree, terminals, changes and a project shell.

## Develop

```sh
CARGO_TARGET_DIR=/c/tmp/deck-target-app pnpm --dir app tauri dev
```

## Test

```sh
cargo test --workspace
pnpm --dir app test
```

## Build the installer

```powershell
scripts/build-installer.ps1
```

Builds `deck-host` in release, bundles it as a sidecar and produces a per-user NSIS installer
under `<CARGO_TARGET_DIR>\release\bundle\nsis\` (default target dir `C:\tmp\deck-target-release`).

## Data

Settings and state live in `%LOCALAPPDATA%\ClaudeDeck`. Uninstalling leaves this folder alone.
