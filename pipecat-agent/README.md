# Kaira realtime agent (Pipecat)

A streaming voice pipeline for testing the **live-agent** architecture and **VAD** —
separate from the Slint harness (which is for *evaluating* engines).

```
mic → Silero VAD → Soniox STT → Gemma-4 (Cerebras/OpenRouter) → Soniox TTS → speaker
```

Persistent WebRTC connection, so it avoids the per-turn HTTP latency the harness
fights (stale keep-alive, TLS setup, TTFB stalls). Silero VAD drives turn-taking
and barge-in (interrupt Kaira mid-sentence).

## Run it (browser test client)

```bash
cd valdi/pipecat-agent
cp .env.example .env        # then fill in SONIOX_API_KEY + OPENROUTER_API_KEY
                            #   (copy the values from Rust-Mobile/.env.local)
uv sync                     # installs pipecat + Soniox/OpenRouter/Silero/WebRTC (pulls torch, ~1st run slow)
uv run bot.py               # serves the WebRTC test client
```

Open **http://localhost:7860**, click **Connect**, allow the mic, and talk. Kaira
greets you, then you're in a live conversation — watch VAD fire as you start/stop
speaking, and try interrupting mid-reply (barge-in).

## Tuning VAD (the point of this test)

In `bot.py`, `VAD_PARAMS`:
- **`stop_secs`** (default 0.6) — silence before your turn ends → the biggest
  responsiveness knob. Lower = snappier but risks cutting you off mid-pause.
- **`start_secs`** (0.2) — how long speech must persist before "you started" fires.
- **`confidence`** / **`min_volume`** — speech-probability and loudness gates
  (raise to ignore background noise).

## Swap models to A/B latency

`.env`: `KAIRA_LLM=groq/llama-3.3-70b-versatile` (or any OpenRouter model).
Default is `google/gemma-4-31b-it` pinned to the Cerebras provider (~0.12s TTFT).

## Next: wire it into the Slint harness

The browser client is the fastest way to test VAD today. To test on-device
(OnePlus/A3), the harness needs a realtime client — a WebSocket transport is the
practical path for the Rust app (WebRTC in Rust is a heavier lift). See the plan
in the harness notes.
