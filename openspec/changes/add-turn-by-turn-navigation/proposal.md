## Why

VOID_AI has an interactive Mapbox map (search, markers, route preview, "Open in Maps" handoff), but it hands real navigation off to the system nav app. To make VOID_AI a *replacement* for the user's main map app — and to give Nova a voice-copilot moment no mainstream nav app has — we need real in-app turn-by-turn guidance that keeps working (with Nova still talking) on spotty or no network.

## What Changes

- Add real in-app **turn-by-turn navigation**: maneuver-by-maneuver guidance, off-route rerouting, live ETA/distance-remaining, and arrival detection — replacing the plan-then-handoff flow for active driving.
- Add an **offline** path: pre-downloaded map tiles + on-device routing + predictive caching so guidance survives dead zones ("meaningful directions with no signal").
- Add a **driving mode** UI: glanceable maneuver banner, speed, distance/ETA, day/night, big touch targets.
- Add **Nova as the driving copilot**: navigation maneuvers are spoken in Nova's voice/personality via on-device TTS (works offline), and Nova can converse *while* navigating (full-duplex "why'd you reroute me?").
- Add **realtime traffic** (online): congestion display and traffic-aware routing/rerouting, degrading gracefully to offline routing when disconnected.
- **BREAKING (behavioral):** for in-app navigation, the `map` op's `route` gains a guidance lifecycle (start/step/reroute/arrive events) rather than a static preview + external handoff. The existing "Open in Maps" handoff remains as a fallback.

Architectural note (details in design.md): this introduces a **native navigation stack alongside the Mapbox GL JS webview** — the webview stays for online map browsing; a Rust-core nav engine (Ferrostar) + on-device routing (Valhalla) + offline tiles (MapLibre/PMTiles) power actual guidance, reusing the existing on-device TTS for Nova's voice.

## Capabilities

### New Capabilities
- `turn-by-turn-navigation`: the core guidance engine — consume a route, track GPS against it, emit maneuver/progress/reroute/arrival events, snap-to-road, off-route detection and rerouting, ETA and distance-remaining.
- `offline-navigation`: on-device routing and offline map rendering from pre-downloaded regional packs; predictive caching of the active route + tiles ahead; graceful online→offline degradation; dead-reckoning across GPS gaps.
- `driving-mode`: the driving UI surface — next-maneuver banner, speed, distance/ETA remaining, lane hints, day/night, minimal-distraction layout, keep-awake/background operation.
- `nova-driving-copilot`: route Nova's voice + personality for spoken guidance (on-device TTS, offline-capable); full-duplex so Nova narrates maneuvers *and* answers questions mid-trip without losing the guidance thread.
- `realtime-traffic`: online congestion display and traffic-aware routing/rerouting, with clean fallback to non-traffic offline routing.

### Modified Capabilities
<!-- No existing OpenSpec specs yet (fresh openspec/specs/). The map/`map`-op behavior is documented in code/UI_CONTRACT.md, not as an OpenSpec capability, so it's captured here as new work rather than a delta. -->

## Impact

- **New deps / stack:** Ferrostar (Rust nav core) + Valhalla (on-device routing) + MapLibre Native (offline tile rendering) + PMTiles/MBTiles offline packs. Evaluate Stadia Maps hosted Valhalla/tiles vs. self-host. Trade-off vs. the turnkey commercial Mapbox Navigation SDK ($/MAU) is decided in design.md.
- **App (`kaira-slint`):** new driving-mode Slint views; a nav-engine module (Rust) bridged to the map surface; offline pack download/storage; continuous background location; reuse `ios_map.rs` map surface and the on-device TTS path.
- **Agent (`pipecat-agent` / Nova):** maneuver events → Nova TTS; duplex guidance-vs-conversation arbitration; `map`-op `route` extended with a guidance lifecycle.
- **Platform:** iOS + Android background location + audio; battery/thermal management; later CarPlay / Android Auto.
- **Data/ops:** map + routing tile pipeline (which regions, update cadence, storage budget on-device); Mapbox token scoping; offline search is a known gap (separate follow-up).
- **Non-goals (this change):** CarPlay/Android Auto, multi-stop/waypoint optimization, EV routing, and offline POI search — tracked as later changes.
