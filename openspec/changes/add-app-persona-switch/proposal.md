## Why

The app is Nova-only: one identity (the constellation orb), one voice, one bot. We've built a second persona server-side — **Kaira**, a calm GP health assistant with her own prompt, voice (Sulafat), and `find_specialist` tool (see `pipecat-agent/personas.py`). This change makes the persona a real, switchable product feature *in the app*: **swipe the agent icon to change agents**, with each persona wearing its own identity — including **Kaira's character avatar** (idle pose + talking animation) in place of Nova's orb.

## What Changes

- **Persona state + swipe** (`kaira-slint`): a horizontal swipe on the agent icon toggles **Nova ⇄ Kaira**; each persona has its own name, accent palette, and agent icon. Tap still connects/disconnects as today.
- **Two-bot wiring (app picks the URL)**: Nova and Kaira run as **separate bot instances** via the existing `KAIRA_PERSONA` env (`nova`|`kaira`). The app holds both bot URLs and connects/reconnects to the selected persona's bot on swipe. **No bot code changes.**
- **Kaira animated avatar**: Kaira's agent icon is her character art — a static **idle** pose that plays an **idle→talking loop while she is speaking** (driven by the existing agent-speaking state). Nova keeps the constellation orb.
- **Assets**: import Kaira's frames — `Aiko-Idle.webp` (idle) + the 11-frame talking loop (`Expanded Animation/Frame 1–11.webp`, 540×675) — into `kaira-slint/assets`, converted to a Slint-friendly format.

## Capabilities

### New Capabilities
- `app-persona-switch`: swipe-to-switch persona in the app, per-persona identity (name, palette, icon), and reconnecting to the selected persona's bot (two-bot model).
- `agent-avatar-animation`: the agent icon can be a character **avatar** with an idle pose + speaking-driven talking animation; per-persona (Kaira = portrait, Nova = orb).

### Modified Capabilities
<!-- None — extends the existing app view surface / realtime connection without changing their specs. -->

## Impact

- **App (`kaira-slint`):** `ui/app.slint` (persona state, swipe gesture on the orb/avatar, per-persona visuals, avatar animation), `src/lib.rs` (persona → bot-URL selection; drive the avatar's speaking state), `src/realtime.rs` (reconnect to the chosen URL). New assets under `assets/`.
- **Bot (`pipecat-agent`):** none — Nova and Kaira are two instances via `KAIRA_PERSONA`. Deploy/`servers.sh` runs both.
- **Config/deploy:** the app needs **two bot URLs** (one per persona); the droplet runs both instances (two ports).
- **Non-goals:** in-session persona switching without a reconnect (voice/prompt/tools are fixed at connect — see design D1); a single-endpoint `?persona=` param (deferred — the two-bot model avoids bot/runner changes); more than two personas (the model is extensible but only Nova + Kaira are in scope); audio-accurate lip-sync (a simple talking loop only).
