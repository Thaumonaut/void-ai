## Context

Today VOID_AI renders an interactive **Mapbox GL JS map in a WKWebView** (`ios_map.rs`) over the Slint "map" view, driven by Nova's `map` op, with a route polyline from the Mapbox Directions API and an "Open in Maps" handoff for actual driving. This is an excellent *online map viewer*, but it cannot do turn-by-turn, and — critically — it is **online-only**: tiles and routes come from Mapbox servers, so no network = blank map + no route.

Assets we already have that de-risk this: a **Rust/Slint** codebase, **on-device TTS** (Piper/Supertonic via sherpa-onnx), the **Nova** agent, GPS (CoreLocation/LocationManager), and working Mapbox/Directions integration.

The hard requirement — "meaningful directions on spotty or no network" — is the forcing function. Offline turn-by-turn needs three things a webview fundamentally cannot do: offline map tiles, on-device routing, on-device voice. We have the third.

## Goals / Non-Goals

**Goals:**
- Real in-app turn-by-turn guidance (maneuvers, rerouting, ETA, arrival).
- Guidance that continues through dead zones (offline tiles + on-device routing + predictive caching).
- Nova's voice/personality as the guidance voice, offline-capable, with converse-while-navigating.
- Realtime traffic when online, degrading gracefully offline.
- Reuse existing assets (Slint UI, on-device TTS, Nova, the webview map for browsing).

**Non-Goals (this change):**
- CarPlay / Android Auto, multi-stop optimization, EV routing, offline POI search (later changes).
- Replacing the online browsing map — the GL JS webview stays for search/preview.

## Decisions

**D1 — Add a native navigation stack alongside the webview (don't force nav into the webview).**
GL JS + Directions are online-only; offline TBT is impossible in that model. So: keep the webview for online browsing, add a native offline-capable nav engine for active guidance. *Alt considered:* build TBT on Directions API `steps` inside the webview — rejected (still online-only; reinvents rerouting/snapping/state-machine).

**D2 — Navigation core: Ferrostar (Rust).** Its core is a Rust crate (nav state machine, maneuver logic, Valhalla/OSRM integration) — it drops into our Rust/Slint app, and its maneuver events are consumed directly by Nova (also Rust-driven). Open, offline-capable, no per-MAU lock.
*Alts:* **Mapbox Navigation SDK** — turnkey (TBT+traffic+offline+voice) but native Swift/Kotlin, vendor lock, and priced per MAU/trip ($0.30/MAU, $0.048/trip beyond free tier); we'd bridge from Rust and lose voice control. Kept as the fast fallback if Ferrostar maturity blocks us. **Roll-your-own** — too much surface area.

**D3 — Routing engine: Valhalla.** On-device offline routing from pre-downloaded, regionally-extractable tiles; first-class Ferrostar backend; C++ so it cross-compiles to iOS/Android. *Alts:* OSRM (fast but heavier to run on-device, less offline-friendly), GraphHopper (JVM — awkward on iOS).

**D4 — Map rendering for nav: MapLibre Native + PMTiles/MBTiles.** Open, renders vector tiles fully offline, no per-load cost; pairs with Ferrostar's UI layer. Option to unify the *browsing* webview on MapLibre GL JS too (shared styles/tiles, drop Mapbox per-load billing). *Alt:* Mapbox GL Native (offline regions exist but commercial + lock-in).

**D5 — Custom Slint driving UI over the Ferrostar Rust core (not Ferrostar's SwiftUI/Compose UI).** The app is Slint; we consume Ferrostar's core events and render a native Slint driving mode. MapLibre Native is embedded like we embedded WKWebView — a positioned native view (or texture) under the Slint surface (reuse the `ios_map.rs` embedding lessons).

**D6 — Voice: reuse on-device TTS; Nova is the personality layer.** Ferrostar emits structured maneuver/progress events → a "guidance narrator" renders them in Nova's voice via on-device TTS (works offline). A duplex arbiter interleaves guidance prompts with conversational turns (guidance is interruptible but never dropped).

**D7 — Online↔offline degradation model.** Online: traffic-aware routing + congestion display + traffic rerouting. Losing signal: predictive-cached route + tiles keep guidance going; rerouting falls back to on-device Valhalla (historical/no-traffic). Same Nova voice throughout. Never a blank screen.

**D8 — Hosted vs self-host tiles/routing: start hosted (Stadia Maps), design for offline packs.** Stadia (Ferrostar's authors) offers hosted Valhalla + MapLibre tiles + offline packs — fastest path; revisit self-hosting for cost/control later.

## Risks / Trade-offs

- **Binary/app size + offline data** (Valhalla tiles, MapLibre native, regional packs) → phase offline behind Phase 2; make packs opt-in per-region with a storage budget.
- **Ferrostar maturity / API churn** (young project) → keep Mapbox Nav SDK as a fallback (D2); isolate behind our own nav trait.
- **Two map renderers** (Mapbox GL JS webview + MapLibre native) → unify on MapLibre over time (D4).
- **Embedding MapLibre Native under Slint** is another native-view integration (like WKWebView) → reuse the `ios_map.rs`/passthrough patterns; validate on both iOS + Android.
- **Traffic requires network** — inherent; the degradation model (D7) makes it a graceful loss, not a failure.
- **Background location + battery/thermal** → use platform background-location modes, adaptive GPS/render cadence.
- **Android parity** — every native piece (MapLibre, Valhalla, background location) must ship on both; budget Android explicitly.

## Migration Plan

- Phase 1 (online TBT): Ferrostar + hosted Valhalla/MapLibre, Nova maneuver voice, Slint driving mode, traffic display. Feature-flagged; "Open in Maps" stays as fallback.
- Phase 2 (offline): regional route/tile packs, predictive caching, offline voice + rerouting. The "no network" capability.
- Phase 3 (polish): traffic rerouting, lane guidance, incidents, alternatives, then CarPlay/Android Auto.
- Rollback: the flag reverts to today's preview + handoff at any point.

## Open Questions

- Hosted (Stadia) vs self-hosted routing/tiles at scale — cost + offline-pack control?
- Which regions to pre-pack, update cadence, and on-device storage budget?
- How does Nova arbitrate guidance vs. conversation (priority, barge-in, ducking)?
- MapLibre-under-Slint embedding: native subview vs. shared-texture (as explored for Servo)? iOS + Android approach.
- Do we unify the browsing map onto MapLibre now, or keep Mapbox GL JS until Phase 2?
