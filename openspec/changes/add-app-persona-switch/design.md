## Context

A persona's voice + system prompt + tools are baked into the bot's realtime (Gemini Live) service when the pipeline is built **at connect time** — they cannot change mid-session. The app is Nova-only today: the agent icon is the constellation orb (`ui/app.slint`), and tapping it drives `on_realtime_toggle → realtime.connect(url)` against a single `/api/offer`. We already have a working server-side persona switch (`KAIRA_PERSONA=nova|kaira`, `pipecat-agent/personas.py`) and Kaira character assets (idle + talking frames, 540×675 WebP). This change surfaces the persona as an app-level, swipeable choice with its own identity and avatar.

## Goals / Non-Goals

**Goals:** swipe to switch agents; each persona its own identity, including Kaira's animated avatar; minimal-to-no bot changes.
**Non-Goals:** mid-session switching; single-endpoint persona routing; >2 personas; real lip-sync.

## Decisions

**D1 — Two bots; the app picks the URL.** Persona is fixed at connect, so switching = reconnect to a bot serving that persona. Running Nova and Kaira as separate instances (existing `KAIRA_PERSONA` env) lets the app simply choose a URL — **zero bot/runner changes**, cleanly isolated from other in-flight bot work. *Alt:* single endpoint + `?persona=` query param — one endpoint, but needs bot + pipecat-runner plumbing to read the param per-connection; deferred.

**D2 — Swipe on the agent icon; reconnect on change.** A horizontal drag past a threshold toggles persona. If currently connected, disconnect + reconnect to the new persona's URL (a persona change is a new session regardless). Tap keeps its current connect/disconnect behavior; long-press keeps its current behavior — the gesture recognizer must disambiguate tap vs long-press vs horizontal drag.

**D3 — Per-persona agent icon: orb vs avatar.** Nova keeps the constellation orb. Kaira renders a **character avatar**: a static idle image that switches to a **frame-cycled talking loop while the agent-speaking flag is set** — the same state that already mutes the mic during playback (`AGENT_PLAYING` in `realtime.rs`). *Alt:* audio lip-sync — out of scope; a short talking loop reads well and is cheap.

**D4 — Assets: convert to PNG, preload frames.** WebP decoding in Slint is build-dependent; convert `Aiko-Idle.webp` + the 11 talking frames to PNG under `assets/kaira/`. Preload frames into an image array and cycle an index on a timer (~8–12 fps) while speaking; show idle otherwise. Keep frames at 540×675 to bound app size.

**D5 — Persona identity is data, not branching UI.** A small per-persona table on the app side — `{ name, accent palette, icon_kind: orb|avatar, bot_url }` — mirrors the server `personas.py`, so a third persona later is a data row, not new UI. The agent-icon component switches on `icon_kind`.

## Risks / Trade-offs

- **Two instances = ~2× server cost** and both must be deployed/healthy. Acceptable for a prototype; revisit with the single-endpoint option (D1 alt) if this scales.
- **Reconnect on swipe drops the session** — no cross-persona continuity. Intended: it's a different agent.
- **Gesture collisions** on the orb (existing tap + long-press vs new drag) → require a clear horizontal-drag threshold before it counts as a swipe.
- **App size** from 11 PNG frames → keep them small; the 2-frame talking option is a fallback if size matters.

## Migration Plan

Additive; Nova stays the default. **Phase 1:** persona table + swipe + per-persona name/palette + reconnect to the chosen URL (orb for both personas). **Phase 2:** Kaira avatar (idle + talking animation driven by the speaking flag). **Phase 3:** second bot instance wired into deploy/`servers.sh`, and the app's two URLs configured.

## Open Questions

- Where do the two bot URLs come from — build-time config, an app settings screen, or discovery?
- Should the collapsed/mini agent strip also show the persona avatar, or just the full orb/avatar?
- Idle micro-animation (subtle breathing/blink) or a fully static idle pose?
- Persist the last-selected persona across launches, or always default to Nova?
