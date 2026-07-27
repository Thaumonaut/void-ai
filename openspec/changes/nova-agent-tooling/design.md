## Context

Nova runs as a Pipecat cascade over WebRTC (Soniox STT → Gemma-4/Cerebras LLM → Soniox TTS). The client (`kaira-slint`) is deliberately keyless — all tools run server-side and stream results (URLs, blocks, pins, images) into the Slint view surface over the RTVI data channel. The system is built and live-verified; this design records the decisions.

## Goals / Non-Goals

**Goals:** Nova drives the views via tools; never dead air during slow fetches; reliable tool-calling; keyless client.
**Non-Goals:** full-duplex/S2S (separate change); rendering rich web pages in-app beyond the map (web/doc stay block-based).

## Decisions

**D1 — Tools are (capability + UI trigger) in one.** Each tool fetches, pushes results to its view via `UiBridge`, and returns a short text summary so Nova reacts. No separate "drive UI" calls for searches. *Alt:* separate tool + explicit UI ops — more round-trips, more model burden.

**D2 — Filler banter is a two-layer safety net.** Layer 1: on function-call-in-progress, immediately TTS one curated in-character line per tool (reliable, model-independent). Layer 2: prompt the LLM to emit a one-liner *with* the tool call and suppress Layer 1 if present. Gemma-4 won't reliably interleave text+tool_call, so Layer 1 is the guarantee. *Rationale:* pure S2S models freeze/go silent during a tool call; this cascade+filler gracefully covers the 1–3s network gap.

**D3 — Cascade (Gemma-4/Cerebras + Soniox), not S2S.** Verified 10/10 tool-calls + 0/4 over-calls, ~0.6s, cheap. Tool reliability + cost beat S2S for Nova's slow-tool workload. *Alt:* S2S — deferred to `add-s2s-fullduplex-mode` as a flag-gated A/B.

**D4 — Keyless client; server-side tools.** Secrets live only on the bot (`.env.local`); the client just renders what's streamed (only the Mapbox `pk.` rides along in a map URL). Simpler, safer client; centralizes tool logic.

**D5 — Transport via `OutputTransportMessageUrgentFrame`, not RTVIServerMessageFrame.** The minimal webrtc-rs client does not do the RTVI client-ready handshake an `RTVIProcessor` waits for, so `UiBridge` queues urgent transport messages directly on the existing `chat` data channel.

**D6 — Web/doc = server-fetch + readability blocks; map = interactive WebView.** The web-article/doc views render simplified block streams (fast on spotty networks). The *map* view diverged from that plan and now uses an interactive Mapbox WKWebView (`ios_map.rs`). This split is intentional: maps need live pan/zoom; articles do not.

## Risks / Trade-offs

- **Stale-bot debugging** (Python has no hot reload) → if tools "don't fire live," verify the running bot's cwd/pid before suspecting the model.
- **Layer 2 filler double-talk** → suppress Layer 1 only when the model actually emits a one-liner.
- **Model over-calling / refusing to show** → hardened prompt ("you HAVE a screen, never say you can't"), temp 0.5.

## Migration Plan

Built incrementally (contract → view surface → tools → prompt/filler → live). Remaining tail is additive polish; no rollback needed. Full-duplex is a separate flag-gated change.

## Open Questions

- Wire client→server view events (user-driven view changes reported back to the bot)?
- Do web/doc views eventually move to a WebView too (per the web-replatform direction), or stay block-based?
