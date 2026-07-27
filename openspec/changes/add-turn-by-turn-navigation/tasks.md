## 1. Foundations & de-risking (spike before committing)

- [ ] 1.1 Spike Ferrostar's Rust core in a scratch crate: build for `aarch64-apple-ios(-sim)` + `aarch64-linux-android`; feed it a canned Valhalla route and log the maneuver/progress event stream
- [ ] 1.2 Decide hosted (Stadia Maps Valhalla+tiles) vs self-host for Phase 1; provision access/keys
- [ ] 1.3 Define a `NavEngine` Rust trait in `kaira-slint` (start/cancel session; event stream: progress, maneuver, reroute, arrival) so Ferrostar is swappable for the Mapbox Nav SDK fallback
- [ ] 1.4 Spike MapLibre Native embedded under the Slint surface on iOS (reuse the `ios_map.rs` / passthrough patterns) and confirm the same approach on Android

## 2. Phase 1 — Online turn-by-turn

- [ ] 2.1 Wire route request → Ferrostar guidance session start from the existing `map` op / a "Start navigation" action
- [ ] 2.2 Feed live GPS (CoreLocation / LocationManager) into the engine; implement snap-to-road via the engine
- [ ] 2.3 Implement off-route detection + reroute (online) and surface reroute events
- [ ] 2.4 Emit + consume progress events: live ETA, remaining distance, current step
- [ ] 2.5 Implement arrival detection + session end
- [ ] 2.6 Keep "Open in Maps" as a fallback path behind a feature flag

## 3. Phase 1 — Driving mode UI (Slint)

- [ ] 3.1 Build the Slint driving-mode surface: next-maneuver banner, speed, distance/ETA, large targets
- [ ] 3.2 Bind the surface to the `NavEngine` event stream
- [ ] 3.3 Day/night appearance
- [ ] 3.4 Keep-awake during guidance; background location + audio so guidance continues with screen off
- [ ] 3.5 Lane / next-step hints when present in route data

## 4. Phase 1 — Nova driving copilot

- [ ] 4.1 Guidance narrator: render maneuver/reroute/arrival events into Nova-voiced prompts via the on-device TTS path
- [ ] 4.2 Duplex arbiter: interleave guidance prompts with Nova conversation; prioritize due maneuvers (interrupt/duck), never drop one
- [ ] 4.3 Extend the `map` op `route` with the guidance lifecycle (start/step/reroute/arrive) so the agent stays in sync
- [ ] 4.4 In-character reroute/arrival narration

## 5. Phase 1 — Traffic display (online)

- [ ] 5.1 Show live congestion coloring along the route + nearby roads
- [ ] 5.2 Use traffic-aware routing for the initial route + ETA
- [ ] 5.3 Indicate clearly when traffic is unavailable

## 6. Phase 2 — Offline

- [ ] 6.1 On-device Valhalla routing from downloaded tiles (build/cross-compile Valhalla for iOS + Android)
- [ ] 6.2 Offline map rendering via MapLibre Native from PMTiles/MBTiles packs
- [ ] 6.3 Region download manager: pick region → download map+routing packs, show size, remove; storage budget
- [ ] 6.4 Predictive caching of the active route's tiles + route data while connected
- [ ] 6.5 Online↔offline degradation: prefer online/traffic routing, fall back to on-device without ending the session or blanking the map
- [ ] 6.6 Dead-reckoning (heading + speed) to bridge GPS gaps (tunnels)
- [ ] 6.7 Verify end-to-end with the device in airplane mode inside a downloaded region

## 7. Phase 3 — Polish

- [ ] 7.1 Traffic-triggered rerouting (offer/switch to faster route mid-trip)
- [ ] 7.2 Route alternatives selection
- [ ] 7.3 Incidents / closures display
- [ ] 7.4 Consider unifying the browsing map onto MapLibre GL JS (retire the Mapbox GL JS webview)
- [ ] 7.5 Scaffold CarPlay / Android Auto (own change)

## 8. Cross-cutting

- [ ] 8.1 Android parity for every native piece (MapLibre, Valhalla, background location) — budgeted explicitly
- [ ] 8.2 Battery/thermal: adaptive GPS + render cadence during guidance
- [ ] 8.3 Scope/restrict the Mapbox (and any new) tokens; move tokens out of source into config
- [ ] 8.4 Tests: scenario coverage from the specs (guidance lifecycle, offline routing/render, degradation, duplex priority)
