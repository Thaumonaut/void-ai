## Why

Nova is a voice agent with a screen — she needs to *drive* the VOID_AI view surface (map, images, shopping, web, docs) with her own tools, and never leave dead air while a slow network tool runs. This captures the built tool + filler-banter system as a spec, and tracks the remaining tail. (Status: Phases 1–3 built and live-verified 2026-07; deferred polish + a few gaps remain.)

## What Changes

- **Server-side tool layer** (`pipecat-agent/tools.py`): 6 LLM tools that fetch, push results into a view over the RTVI data channel, and return a short summary so Nova reacts (not recites): `search_places`, `get_directions`, `search_web`, `search_images`, `search_products`, `start_navigation`. *(Built.)*
- **In-character filler banter** to cover 1–3s tool latency: instantly speak one curated in-character line the moment a tool is called (Layer 1, reliable); optionally an LLM one-liner emitted with the call (Layer 2, when the model cooperates). *(Layer 1 built.)*
- **RTVI UI-control contract** (`kaira-slint/UI_CONTRACT.md`) + client apply (`apply_ui_control`) driving the Slint `VS` view surface. *(Built.)*
- **Keyless client / server-side tools** on the cascade (Gemma-4 on Cerebras, tool-calling verified 10/10; Soniox STT/TTS). *(Built.)*
- **Remaining tail:** client→server view events (user-driven view changes back to the bot); deferred UI polish (4-side switcher, real remote thumbnails, in-chat image thumbnails, Nova-pause/mute dock, mini-constellation in collapsed strip); reconcile the web-article view (server-fetch + readability blocks) with the interactive-Mapbox WebView added for the map view.

## Capabilities

### New Capabilities
- `agent-view-tools`: Nova's server-side tools that fetch content and drive the corresponding view (map/images/shopping/web/docs), returning a summary for the agent to react to; thin UI-only actions (`set_view`, `collapse`, `close_view`); the RTVI UI-control contract.
- `agent-filler-banter`: the latency-covering behavior — the agent speaks an in-character line the instant a tool is invoked so there is never dead air during a slow fetch.

### Modified Capabilities
<!-- None — no prior OpenSpec specs exist for these. -->

## Impact

- **Agent (`pipecat-agent`):** `tools.py` (6 tools + schemas), `ui.py` (`UiBridge` over RTVI), `bot.py` (SYSTEM_PROMPT, filler wiring, tool registration). Backends: Mapbox (places/directions), Tavily (web/images), SerpApi (shopping).
- **App (`kaira-slint`):** `UI_CONTRACT.md`, `realtime.rs` (`UiControlCb` / `server-message`), `lib.rs` `apply_ui_control` → `VS` models, `ui/app.slint` view surface.
- **Interaction with other work:** the map view now uses an interactive Mapbox WKWebView (`ios_map.rs`) rather than the block-only rendering originally planned — this proposal's specs treat the map view as webview-backed while web/doc views stay block-based.
- **Non-goals:** full-duplex/S2S (see `add-s2s-fullduplex-mode`); offline behavior (see `add-turn-by-turn-navigation`).
