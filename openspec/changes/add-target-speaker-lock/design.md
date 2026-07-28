## Context

Nova's cascade is `mic → transport.input() (VAD) → Soniox STT → user_aggregator → Gemma → Soniox TTS → speaker`, with Silero VAD + a smart-turn analyzer driving turn start/stop (`bot.py`). Every VAD-detected turn is transcribed and answered, regardless of who spoke. The client already mutes the mic while Nova plays (no voice barge-in — `AGENT_PLAYING` in `realtime.rs`), which simplifies gating: we only ever need to judge *user* turns, and echo is not in scope.

The verification primitive already exists on-device: the `sherpa-onnx` crate (shipped on Android) exposes `SpeakerEmbeddingExtractor` (audio → embedding) and `SpeakerEmbeddingManager` with `add(name, emb)` / `verify(name, emb, threshold)` / `search(emb, threshold)`. The same models (WeSpeaker / 3D-Speaker CAM++, ONNX, tens of MB) have Python bindings for the bot. So this is an integration + UX problem, not a modeling one.

## Goals / Non-Goals

**Goals:** enroll once; act only on the enrolled user's speech; ignore other people's separate utterances; never lock the real user out (fail-open); a visible, disable-able lock; a path to on-device so only the user's audio leaves the phone.
**Non-Goals:** separating speech that overlaps the user (extraction); multi-user/household; gating inside the S2S engines' internal audio loop; anti-spoofing.

## Decisions

**D1 — Verification-gating, not extraction (for v1).** Embed each completed turn and compare to the enrolled print; drop non-matches. It cleanly handles the common case (people take turns; a bystander's separate utterance is rejected) and the phone-held-near-the-face near-field advantage covers a lot. True overlap (someone talking *over* the user) muddies a single embedding and is explicitly deferred to a later extraction escalation — there is no strong open pretrained target-speaker-extraction model, so it's a real project, not a config change.

**D2 — Gate at the turn boundary, before STT.** Insert a `FrameProcessor` that buffers a turn's audio and, on turn-stop, verifies before letting the turn reach Soniox. A non-match is dropped → no STT, no LLM, no TTS, no cost. *Alt:* gate after STT on the transcript — simpler but pays for transcription of everyone in the room; rejected on cost + it still leaks others' speech to the provider.

**D3 — Fail-open with floor-holding hysteresis.** False-reject (ignoring the real user) is the UX-killer; false-accept (an occasional stray) is a shrug. So: audio shorter than `MIN_ENROLL_MS` or below a confidence floor → **accept**. Once a turn is accepted, hold the floor: for `HOLD_MS` afterward, gate the next turns at a **looser** threshold so clipped continuations ("stop", "no—the other one") pass. This directly answers the repo's prior scar (a mic energy-gate once "wedged shut → no transcription"): the gate must degrade toward *listening*, never toward *deaf*.

**D4 — Server-side first, on-device as the end state.** The gate lives in the bot (`speaker.py` + a cascade `FrameProcessor`) for v1: fastest to tune, no client build changes, works today. On-device is the target end state (only the user's audio is transmitted; engine-agnostic across cascade/Gemini/Ultravox; private) — trivial on Android (sherpa already linked) but blocked on iOS by the missing sherpa/onnxruntime iOS build, which is scoped here but deferred. The enrolled embedding is portable between the two homes (same model), so moving the gate later doesn't invalidate enrollments.

**D5 — Enroll through the real capture path, persist as data.** Enrollment records through the same VPIO/AEC chain the user runs through (mic coloring shifts the embedding, so enrolling through a different path degrades matching). Store the embedding (a short float vector) as first-class per-user data via the existing `files_dir()` store, keyed like `add-user-memory`. Deliver it to the bot per-connection (with the offer or over the data channel). Re-enroll/reset from Settings; a corrupt/absent print → lock **disabled** (behaves like today).

**D6 — Thresholds are tunable and observable.** `VERIFY_THRESHOLD`, `HOLD_MS`, `MIN_ENROLL_MS`, and the hold-threshold delta are env/config, not hard-coded — target-speaker EER varies by model and room. Log each decision (score, threshold, accepted/dropped, reason) so thresholds can be tuned against real recordings, and surface a lightweight lock indicator to the user.

## Risks / Trade-offs

- **False-reject frustration** → D3's fail-open + hysteresis; bias the default threshold toward acceptance; make the lock trivially toggle-off.
- **Overlap is unsolved** → documented non-goal (D1); the near-field advantage + turn-taking assumption carry v1; extraction is a later change.
- **Short-utterance instability** → embeddings of <~1s are noisy; treat sub-threshold-duration turns as accept (D3), not reject.
- **Enrollment/runtime path mismatch** → enroll through VPIO (D5); if the capture path changes, prompt a re-enroll.
- **iOS on-device blocked on a build** → server-side ships value now (D4); the sherpa-iOS lift is isolated and deferred.
- **Latency** → embedding compute is ~10–30ms; the real cost is waiting for a near-complete turn before deciding, adding a little to turn latency in cascade. Acceptable; personal-VAD (frame-level) is a future optimization if it bites.
- **Privacy of the voiceprint** → it's biometric-adjacent; on the server it lives with the (per-user) memory store; document where it lives and that on-device is the privacy end state.

## Migration Plan

Additive and default-safe. Un-enrolled or lock-off ⇒ behaves exactly like today (no gate). **Phase 1:** `speaker.py` + cascade `FrameProcessor`, fixed enrolled print from a file, tune threshold/hysteresis against real noisy recordings (server-side, cascade only). **Phase 2:** onboarding enrollment step + persistence + per-connection delivery + lock indicator/toggle. **Phase 3:** move the gate on-device (Android first via the existing sherpa-onnx). **Phase 4 (deferred):** sherpa/onnxruntime iOS build for on-device iOS; and, only if needed, target-speaker extraction for true overlap.

## Open Questions

- Which embedding model — WeSpeaker CAM++ vs 3D-Speaker vs SpeechBrain ECAPA — best trades EER against size/latency in real rooms, and does one model serve both server and on-device?
- Deliver the enrolled print with the `/api/offer` payload or over the data channel post-connect? (Offer is simplest; data channel avoids putting biometric data in a query.)
- Does the smart-turn buffer already expose the turn's raw audio to the `FrameProcessor`, or must the gate buffer `InputAudioRawFrame`s itself between VAD start/stop?
- Should a rejected turn ever surface anything to the user ("ignored — not your voice"), or stay silent to avoid distraction?
- Multiple enrolled profiles later (partner, family) — worth shaping the store for now, or strictly single-user until asked?
