## Why

Nova answers whoever talks. In a quiet room that's fine; in a café, an open office, a car with passengers, or a room with a TV on, she responds to strangers, bystanders, and background speech — which is unusable for a personal assistant you carry around. The fix is a **target-speaker lock**: enroll the user's voice once, then only act on speech that matches it and ignore everyone else. The primitive is already in the tree — the `sherpa-onnx` crate we ship on Android exposes on-device speaker embeddings + verification (`SpeakerEmbeddingExtractor`, `SpeakerEmbeddingManager.verify(name, embedding, threshold)`), and the same model has Python bindings for the bot. (Status: proposed.)

## What Changes

- **Voice enrollment** (`kaira-slint`): a one-time "teach Nova your voice" step folded into the existing onboarding flow — capture ~8s of the user through the same VPIO/AEC path they'll run through, compute a speaker embedding, and persist it. Re-enroll / reset available from Settings.
- **Turn-level speaker gate** (`pipecat-agent`): between "a turn was detected" (Silero VAD + smart-turn) and "run the LLM", embed the turn's buffered audio and verify it against the enrolled print. A non-match is **dropped before Soniox STT** (no transcription, no LLM, no reply, no cost). Applies to the cascade pipeline where a per-turn gate is clean.
- **Fail-open, with hysteresis**: on short/low-confidence audio the gate **accepts** (a stray reply beats ignoring the real user); once the user "has the floor," a looser threshold holds for a few seconds so clipped continuations ("stop", "no, the other one") aren't rejected. A user-visible **lock indicator** shows when Nova is filtering, and the lock can be toggled off.
- **On-device end state (phased)**: run the gate in the client so only the user's speech is ever transmitted — engine-agnostic (cascade / Gemini / Ultravox), bandwidth-saving, private. Trivial on Android (sherpa-onnx already linked); the iOS build (sherpa/onnxruntime) is scoped but deferred, so the shipping path is **server-side gating first** (cascade), on-device second.

## Capabilities

### New Capabilities
- `voice-enrollment`: capture, compute, and persist the user's speaker embedding (voiceprint) during onboarding, through the real capture path, with a reset/re-enroll affordance and a graceful un-enrolled state.
- `target-speaker-lock`: gate turn-taking on speaker verification — act only on speech matching the enrolled print, drop non-matching turns before transcription, fail open on uncertainty with floor-holding hysteresis, and surface/allow-disabling the lock.

### Modified Capabilities
<!-- None as OpenSpec specs. Interacts with the cascade turn logic (nova-agent-tooling / add-s2s-fullduplex-mode) and the onboarding flow (app-onboarding-and-permissions) without changing their specs. -->

## Impact

- **Agent (`pipecat-agent`):** a new speaker-gate `FrameProcessor` inserted before Soniox STT in the cascade pipeline (`bot.py`); a `speaker.py` wrapping the embedding model (sherpa-onnx-python or SpeechBrain ECAPA); the enrolled print delivered per-connection (data channel or with the offer). Threshold/hysteresis are env-tunable.
- **App (`kaira-slint`):** an enrollment step in the onboarding overlay (`ui/app.slint`) + capture/compute glue (`src/`), reusing the VPIO capture path (`ios_vpio.rs`) and, for on-device gating, the `sherpa-onnx` speaker API; a lock indicator + Settings toggle/reset; persistence of the embedding via the existing `files_dir()` store.
- **Dependencies:** `app-onboarding-and-permissions` (hosts the enrollment step + already owns mic permission). Reuses the `add-user-memory` per-user store shape for keying the print to a user. Independent of `secure-bot-endpoint` for the single-user case.
- **Non-goals:** target-speaker **extraction/separation** for heavy overlapping speech (VoiceFilter-Lite-style — a later escalation only if verification-gating proves insufficient); multi-enrolled-speaker / household support; speaker gating inside the S2S engines' own audio path (covered only via the on-device-before-transmit route); anti-spoofing / replay-attack defense.
