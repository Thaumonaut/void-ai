## Context

From a 4-agent landscape review (mid-2026): true full-duplex and reliable tool-calling barely co-exist. Nova's tools are slow network fetches (1–3s), which pure S2S models handle poorly (they freeze/go silent), while the cascade's filler banter covers the gap gracefully. So the cascade is genuinely well-suited — the real S2S prize is naturalness/barge-in, not latency. Priorities: reliable tools > latency > cost > Pipecat compat.

## Goals / Non-Goals

**Goals:** a flag-gated S2S mode that keeps tools working, to A/B naturalness/barge-in against the cascade; a barge-in upgrade for the cascade; cascade stays default.
**Non-Goals:** replacing the cascade; self-host S2S; switching away from Pipecat.

## Decisions

**D1 — Flag-gate S2S behind `KAIRA_MODE`; cascade is default.** One code path swaps the LLM/transport layer; tools + `UiBridge` are reused. Lets us ship the cascade and evaluate S2S without risk.

**D2 — First challenger: Ultravox.** Won Pipecat's own tool-use benchmark (beat GPT-Realtime/Gemini/Nova Sonic/Grok), native Pipecat S2S, ~150ms, open weights (self-host later). *Risk:* freezes during a tool call (placeholder + inject result as text) — validate on slow tools.

**D3 — Fallback: AWS Nova Sonic.** Async tool calling resolves without breaking audio → best fit for slow tools; cheapest S2S (~$0.015/min); native Pipecat. *Downsides:* ~1.1s TTFT, ~8-min session cap (Pipecat auto-continues).

**D4 — Cascade barge-in upgrade (no new keys).** Pipecat smart-turn / semantic-VAD for natural turn-taking; optional Cartesia Ink STT + Sonic-3 TTS for lower latency later.

**D5 — Avoid list.** Gemini Live (tools collapse under barge-in, 36% APR); LiveKit/Vapi/Retell (replace Pipecat); open full-duplex Moshi/Nemotron VoiceChat/PersonaPlex (no/weak tools). Silicon vendors (Cerebras/Groq/etc.) are cascade accelerators, not S2S models.

**D6 — Ultravox = its backbone's reasoning.** Frozen text LLM + speech adapter (distilled to match backbone logits). Managed fast tier ≈ Llama-3.1-8B (weaker than Gemma-4-31b); 70B/GLM-4.6 backbones ≥ Gemma. Domain accuracy comes from choosing a stronger/domain backbone + tool/RAG grounding, not adapter fine-tuning (LLM is frozen). Fine-tuning requires self-host.

## Risks / Trade-offs

- **Tool-freeze on slow tools (Ultravox)** → validate live against maps/web/shopping; fall back to Nova Sonic's async tools.
- **Cost 5–20× the cascade** → cascade stays the shipping default; S2S is opt-in.
- **Weaker reasoning on fast managed S2S tiers** → treat as a latency↔accuracy dial; pick backbone per need.
- **Session caps / TTFT (Nova Sonic)** → rely on Pipecat auto-continue; measure TTFT.

## Migration Plan

1. Cascade + barge-in upgrade (default, no new keys).
2. `KAIRA_MODE=ultravox` A/B live over WebRTC; measure tool reliability on slow tools, latency, naturalness.
3. If Ultravox tool-freeze is bad, wire Nova Sonic and A/B that.
4. Keep cascade default regardless; document when to flip to S2S.

## Open Questions

- Does Ultravox's tool-freeze feel acceptable on Nova's 1–3s tools, or is Nova Sonic's async model required?
- Which cascade barge-in stack (Pipecat semantic-VAD vs. Cartesia Ink/Sonic-3) is worth the added cost/keys?
