## Why

The app has almost no feedback states: multi-second tool calls show nothing changing (reads as "broken"); failed image/tile fetches leave permanent colored placeholders; empty views render `FOUND 0 · ""`; there's no offline indication and no network monitoring at all. On the target's spotty network these failure paths are guaranteed. (Audit P0/P1 — the connection-status half is handled in `harden-realtime-connection`.)

## What Changes

- **"Nova is working" state**: a visual cue (skeleton/shimmer/status pill) in the target view while a slow tool runs — extend the UI-control contract with an optional `working` op the bot fires before a slow tool.
- **Error / failed-fetch states**: per-slot load state (loading/loaded/failed) for images, the map tile, and products, with a broken-image glyph + tap-to-retry; a "map unavailable" affordance instead of a bare green box.
- **Empty states**: per-view empty state (view icon + "Ask Nova to search…"); suppress `FOUND 0`/`0 stores` headers when empty.
- **Offline detection**: a reachability listener → a global "offline" banner; explanatory state on the connect action when offline.

## Capabilities

### New Capabilities
- `ui-feedback-states`: every view communicates loading, working, error, empty, and offline conditions clearly instead of silently showing a stale/placeholder or broken layout.

## Impact

- `kaira-slint/ui/app.slint` (per-view states, empty states, offline banner), `src/lib.rs` (`spawn_image_fetch` load-state tracking, reachability), `UI_CONTRACT.md` + `pipecat-agent` (`working` op). A platform reachability signal (iOS + Android) is new.
- Non-goals: connection-status truthfulness (done in `harden-realtime-connection`); offline routing/maps (nav change).
