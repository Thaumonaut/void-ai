# CLAUDE.md — orientation for AI agents

Read this first. It's the map of the repo + the non-obvious things that will save you hours.

## What this is
**VOID_AI** — a real-time, voice-first AI agent app. A native Rust/Slint client talks over WebRTC
to a Python (Pipecat) bot that runs full-duplex speech-to-speech via **Gemini Live** and drives the
app's view surface (map / web / images / shopping) with a tool layer. Two swappable personas:
**Nova** (sassy assistant) and **Kaira** (calm GP health assistant). See `README.md` for the pitch.

## The two things that matter
| | Path | Language | Runs |
|---|---|---|---|
| **App (client)** | `kaira-slint/` | Rust + Slint | iOS · Android · desktop |
| **Bot (backend)** | `pipecat-agent/` | Python (Pipecat) | local `:7860` / droplet `:8080`+`:8081` |

Everything else at the top level (`compare/`, `model-test/`, `soniox-*`, `pipecat-bench/`,
`voicelab/`, `VOID_ai/`) is experiments/benchmarks — ignore unless asked.

## Key files
- `pipecat-agent/bot.py` — the pipeline. `KAIRA_MODE` picks `cascade` (Soniox+Gemma+Soniox) vs
  `gemini` (Gemini Live S2S). Reads env from `../.env.local` then `./.env`. Persona selected by
  `KAIRA_PERSONA` at import (sets `SYSTEM_PROMPT` / `PERSONA_VOICE` / `PERSONA_TOOLS`).
- `pipecat-agent/personas.py` — the persona registry (`get_persona`): Nova's prompt lives in
  `bot.py`; Kaira's full GP prompt + voice + tools live here.
- `pipecat-agent/tools.py` — the tool layer (`NOVA_TOOLS`, `KAIRA_TOOLS`, `register_tool_handlers`).
  Place/clinic search uses **Google Maps via SerpApi** (`engine=google_maps`, anchored to the
  user's `ll=@lat,lng`) + a haversine distance filter, with Mapbox as fallback.
- `pipecat-agent/ui.py` — `UiBridge`: how the bot drives the app's views over the RTVI data channel
  (contract in `kaira-slint/UI_CONTRACT.md`).
- `kaira-slint/ui/app.slint` — the whole UI. `Persona` global = active agent (name/accent/index);
  `VS` global = view surface; `Pal` = palette. Swipe the agent icon → `persona-switched`.
- `kaira-slint/src/lib.rs` — Slint↔Rust glue. `bot_url_for(index)` picks the per-persona backend URL.
- `kaira-slint/src/realtime.rs` — the WebRTC client (offer → `/api/offer` → answer; data channel).

## Build & run
```bash
# Bot (local): browser test client at http://localhost:7860
cd pipecat-agent && uv sync && KAIRA_MODE=gemini uv run bot.py
# two personas at once: add --host 0.0.0.0 --port 7861 + KAIRA_PERSONA=kaira for the 2nd

# App (desktop, fast UI loop — realtime is mobile-only):
cd kaira-slint && cargo run

# App (iOS sim): xcodebuild -project KairaSlint.xcodeproj -scheme KairaSlint \
#   -sdk iphonesimulator -destination 'platform=iOS Simulator,id=<UDID>' \
#   -derivedDataPath build_sim build   → simctl install/launch
# App (iOS device): same with -sdk iphoneos, -destination 'platform=iOS,id=<UDID>',
#   -allowProvisioningUpdates → xcrun devicectl device install/launch  (team is Automatic)

# Deploy both bots to the droplet (Nova :8080 cascade, Kaira :8081 gemini):
#   rsync pipecat-agent/ → root@<ip>:/opt/voidai/ ; docker build ; docker run --network host …
#   full recipe in pipecat-agent/deploy/README.md
```
Secrets: `pipecat-agent/.env` (+ repo-root `.env.local`), both gitignored. Copy `.env.example`.

## Hard-won gotchas (don't rediscover these)
- **Gemini Live model id:** the Live/native-audio model is `gemini-3.1-flash-live-preview` (there is
  **no** 3.5/3.6 *live*). Import is `pipecat.services.google.gemini_live.llm` (the package `__init__`
  is empty). `Settings` default model is the 2.5 preview — pin `model` with the `models/` prefix.
- **Gemini Live multi-turn** needs two things or it goes deaf: (1) tools **only** on the service
  (`tools=`), NOT also in the `LLMContext` — else it reconnects and drops mic audio; (2)
  `vad=GeminiVADParams(disabled=True)` so the local Silero VAD drives turns (running both VADs +
  the client muting mic during playback breaks turn 2+).
- **`LLMContext(tools=None)` throws** — omit the arg for "no tools" (the service accepts `None`).
- **Persona = fixed at connect** (voice/prompt/tools baked in). Switching = reconnect to that
  persona's bot. Two-bot model: app picks the URL by persona index.
- **iOS sim** reaches the Mac's `localhost`; drive its UI with `idb ui swipe/tap` (but idb's
  synthetic swipe is flaky — prefer the in-app Settings→Agent toggle for deterministic tests).
  Repoint the sim's backend by writing `realtime_url*.txt` into its app container `Documents/`.
- **Place search "across the country" bug:** Mapbox `proximity` only *biases*. Fixed by using
  Google Maps (`ll`) + a hard distance filter, and `_geocode` now picks the nearest candidate.

## Conventions
- Idiomatic Rust matching the surrounding code; `objc2` for iOS FFI; keep deps cross-compile-friendly.
- Feature work is tracked in `openspec/changes/` (proposal + design + tasks + spec deltas);
  validate with `openspec validate <change> --strict`.
