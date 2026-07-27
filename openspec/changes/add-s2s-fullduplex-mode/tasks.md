## 1. Flag-gated S2S scaffolding (DONE)

- [x] 1.1 `run_bot` branches on `KAIRA_MODE` (`cascade` default / `ultravox` / `realtime` / `s2s`)
- [x] 1.2 Ultravox path: `UltravoxRealtimeLLMService` + `LLMContextAggregatorPair`, drop STT/TTS, reuse `NOVA_TOOLS` + `UiBridge`, `speak_filler=False`
- [x] 1.3 `ULTRAVOX_API_KEY` in `.env.local`; imports/compile clean

## 2. Cascade barge-in upgrade (default engine)

- [ ] 2.1 Add Pipecat smart-turn / semantic-VAD barge-in to the cascade (no new keys)
- [ ] 2.2 (Optional) evaluate Cartesia Ink STT + Sonic-3 TTS for lower latency

## 3. Ultravox A/B — live evaluation

- [ ] 3.1 Run `KAIRA_MODE=ultravox` live over WebRTC end-to-end
- [ ] 3.2 Validate tool-calling against the SLOW tools (maps/web/shopping) — does the tool-freeze feel acceptable?
- [ ] 3.3 Measure latency, naturalness/barge-in, and reasoning quality vs. the cascade
- [ ] 3.4 Pick the managed backbone tier (latency↔accuracy dial)

## 4. Fallback engine (if Ultravox tool-freeze is bad)

- [ ] 4.1 Wire AWS Nova Sonic (`AWSNovaSonicLLMService`, async tool calling) behind `KAIRA_MODE`
- [ ] 4.2 Handle the ~8-min session cap (Pipecat auto-continue) and measure ~1.1s TTFT
- [ ] 4.3 A/B Nova Sonic vs. Ultravox vs. cascade on the same tools/prompt

## 5. Decision + docs

- [ ] 5.1 Record the A/B results and the recommendation (keep cascade default; when to flip to S2S)
- [ ] 5.2 Document how to run each mode and the cost/latency/tool trade-offs
