## Why

Nova's cascade (Soniox + Gemma-4/Cerebras + Soniox) is cheap, ~0.6s, and has the best tool reliability — but it lacks true barge-in and speech-to-speech naturalness. We want to A/B a full-duplex/S2S mode *without* losing reliable tool-calling, and keep the cascade as the shipping default. (Status: the Ultravox A/B is wired behind a flag; live evaluation + a fallback engine + a cascade barge-in upgrade remain.)

## What Changes

- **Flag-gated realtime/S2S mode** via `KAIRA_MODE` (`cascade` default / `ultravox` / `realtime` / `s2s`): the bot swaps the STT→LLM→TTS cascade for a single S2S service while keeping the same tools + `UiBridge`. *(Ultravox path wired; not yet live-tested.)*
- **Keep tools working across modes:** S2S services must still fire Nova's tools; slow (1–3s) tools must not hang the audio.
- **Cascade stays the default** and gains a **barge-in / semantic-VAD** upgrade (no new keys) for more natural turn-taking.
- **Evaluate + choose** the S2S challenger: start with **Ultravox** (won Pipecat's tool-use benchmark) and fall back to **AWS Nova Sonic** (async tool calling — best fit for slow tools) if Ultravox's tool-freeze is bad on slow fetches.

## Capabilities

### New Capabilities
- `s2s-realtime-mode`: an alternative, flag-selected full-duplex conversation engine that preserves Nova's tool-calling and view-driving, runs behind `KAIRA_MODE`, and coexists with the cascade default.

### Modified Capabilities
<!-- None as OpenSpec specs; the cascade tool behavior is specified in nova-agent-tooling. -->

## Impact

- **Agent (`pipecat-agent`):** `bot.py` `run_bot` branches on `KAIRA_MODE`; S2S path uses `UltravoxRealtimeLLMService` (+ `LLMContextAggregatorPair`, no STT/TTS, `speak_filler=False`); reuse `NOVA_TOOLS` + `UiBridge`. New keys: `ULTRAVOX_API_KEY` (present); AWS creds if Nova Sonic is added.
- **Cost/latency:** S2S is ~5–20× the cascade's audio cost; cascade remains default for cost. Ultravox managed ≈ Llama-3.1-8B backbone (weaker reasoning than Gemma-4-31b) — a latency↔accuracy dial.
- **Tool behavior across S2S engines:** Ultravox "freezes" (placeholder + inject result as text) during a tool call → must validate against slow tools; Nova Sonic resolves tools async (better for slow tools) but ~1.1s TTFT + ~8-min session cap.
- **Non-goals:** self-host S2S (Qwen3-Omni / Step-Audio 2 — R&D only); replacing Pipecat (avoid LiveKit/Vapi/Retell); Gemini Live (tools collapse under barge-in).
