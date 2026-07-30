## 1. Server-side gate (cascade) — prove it works

- [ ] 1.1 `pipecat-agent/speaker.py`: wrap a speaker-embedding model (sherpa-onnx-python or SpeechBrain ECAPA) — `embed(pcm) -> vector`, `verify(vector, print, threshold) -> (bool, score)`; load model + choice via env
- [ ] 1.2 `SpeakerGate` `FrameProcessor` (`bot.py`): buffer a turn's `InputAudioRawFrame`s between VAD start/stop, embed on turn-stop, verify against the enrolled print, and DROP the turn (don't forward to Soniox STT) on a non-match
- [ ] 1.3 Insert the gate before `stt` in the cascade pipeline only (S2S paths untouched); no-op when no print is configured
- [ ] 1.4 Fail-open: accept turns shorter than `MIN_ENROLL_MS` or below the confidence floor; never drop on uncertainty
- [ ] 1.5 Floor-holding hysteresis: after an accept, gate at a looser threshold for `HOLD_MS`; reset on expiry
- [ ] 1.6 Env-tunable `VERIFY_THRESHOLD` / `HOLD_MS` / `MIN_ENROLL_MS` / hold-delta; log every decision (score, threshold, accepted/dropped, reason)
- [ ] 1.7 Tune against a real noisy multi-speaker recording; record chosen defaults + EER notes

## 2. Enrollment + persistence + delivery

- [ ] 2.1 `kaira-slint`: "teach Nova your voice" step in the onboarding overlay (`ui/app.slint`) — prompt, ~8s capture, retry-on-unusable
- [ ] 2.2 Compute the embedding from the sample through the live VPIO/AEC path (`src/`), or (interim) send the sample to the bot to embed
- [ ] 2.3 Persist the voiceprint via `files_dir()` (keyed per-user like `add-user-memory`); reset/re-enroll from Settings
- [ ] 2.4 Deliver the enrolled print to the bot per-connection (with the offer or over the data channel); bot loads it into `SpeakerGate`
- [ ] 2.5 Un-enrolled / corrupt print → lock disabled (identical to today)

## 3. Lock UI

- [ ] 3.1 Lock indicator on the Talk surface (active / inactive) driven by whether a print is loaded + the toggle
- [ ] 3.2 Settings toggle to disable/enable the lock; disabling makes every turn answerable regardless of speaker

## 4. On-device gate (Android first)

- [ ] 4.1 Move the gate into the client using the `sherpa-onnx` `SpeakerEmbeddingExtractor` / `SpeakerEmbeddingManager` (already linked on Android): gate the mic pump so only matching turns are transmitted
- [ ] 4.2 Ship/point-to the embedding ONNX model on Android; verify parity of accept/drop decisions vs the server gate
- [ ] 4.3 Engine-agnostic check: on-device gate works identically under cascade, Gemini, and Ultravox (only the user's audio leaves the device)

## 5. Deferred (scoped, not in this change's core)

- [ ] 5.1 sherpa/onnxruntime iOS build so the on-device gate runs on iOS (the primary target)
- [ ] 5.2 Target-speaker EXTRACTION (VoiceFilter-Lite-style) for true overlapping speech — only if verification-gating proves insufficient in real use

## 6. Verify

- [ ] 6.1 With a print enrolled + lock on: a bystander's separate utterance is dropped (no transcription, no reply)
- [ ] 6.2 The enrolled user is answered normally, including short follow-ups within the hold window
- [ ] 6.3 Un-enrolled or lock-off: behavior is identical to today (every turn answered)
- [ ] 6.4 A too-short/too-quiet turn is accepted (fail-open), never silently dropped
