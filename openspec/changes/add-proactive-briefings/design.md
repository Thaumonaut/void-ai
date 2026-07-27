## Context

Nova's pipeline runs the LLM only in response to a user turn: mic → VAD → STT → `LLMRunFrame` → LLM → TTS. There is no path for Nova to speak on her own. Proactivity needs two things that don't exist yet: a way to trigger a Nova turn without user speech, and a way to reach the user when no session is connected. This change adds both, then builds briefings and nudges on top.

## Goals / Non-Goals

**Goals:** Nova can initiate — a scheduled briefing, a timely leave-by nudge, a reminder that fires — without being annoying.
**Non-Goals:** background live voice (iOS won't allow it); smart-home; broadcast; a rich notification-settings UI.

## Decisions

**D1 — Two delivery channels, chosen by connection state.** *Connected/foreground:* inject a synthetic proactive event that triggers a bot turn, so Nova speaks and paints a view live. *Not connected:* send a push (APNs/FCM) whose tap opens the app to the briefing/nudge. One scheduler, two sinks. *Alt:* push-only always — loses the magic of her just *saying* it when you're already in the app.

**D2 — Scheduling lives on the droplet, not the device.** Server-owned jobs survive app kill and fire reliably; device timers don't. This is the same reason `add-daily-assistant-tools` pushes timer/reminder ownership to the bot. The scheduler is the single home for all fire times (briefings, reminders, timers, leave-by).

**D3 — Leave-by math, recomputed late.** `nudge_at = event_start − travel_time − prep_buffer`, where `travel_time` comes from `get_directions(home_or_current → event_location)`. Compute an initial estimate when the event is seen, then recompute close to fire time so live traffic is reflected. Requires the event to have a resolvable location and the user's origin (profile home/work from `add-user-memory`, or live GPS).

**D4 — Quiet hours and frequency caps are first-class, not polish.** The failure mode that kills a proactive assistant is annoyance. Respect a DND window; cap nudges per hour/day; batch what can wait into the next briefing rather than firing separately. A one-tap "stop reminding me about this" must exist.

**D5 — Background is push-only; live proactivity requires foreground.** iOS won't keep a WebRTC voice session alive in the background, so spoken proactivity only happens when the app is foregrounded/connected. Everything else degrades to a push. Ship foreground proactive turns first (no OS-push entitlements needed); add APNs/FCM as a second phase.

**D6 — Proactive speech never collides with the user.** A proactive turn must not fire mid-user-utterance or step on an in-progress exchange; it waits for a clear moment (or defers to push). Reuses the existing VAD/turn state.

## Risks / Trade-offs

- **Annoyance / erosion of trust** → D4 (quiet hours, caps, easy opt-out) is load-bearing; under-deliver rather than over-nudge.
- **iOS background limits** → accept push-only in background (D5); don't promise live background voice.
- **Push infra cost/complexity** (APNs/FCM entitlements, token custody) → phase it; foreground-only proactivity is useful and ships first.
- **Leave-by wrongness** (bad location, stale traffic) → recompute late (D3); prefer "leave ~in 10" phrasing over false precision.
- **Waking the user at a bad time** → schedule against quiet hours and local time zone.

## Migration Plan

Phase 1: scheduler + foreground proactive turn + on-demand briefing (no OS push, no new entitlements) — immediately useful in-app. Phase 2: scheduled briefing + reminder/timer fires with APNs/FCM background push. Phase 3: leave-by nudges (needs location + traffic recompute). Phase 4 (optional): location triggers via `ios_location.rs`. Each phase is independently shippable; quiet-hours/caps land with Phase 1.

## Open Questions

- Pure local timers: server-scheduled + push, or a hybrid with an in-app timer when foregrounded?
- How much of the briefing does Nova speak vs show (react-don't-recite suggests: show the agenda, say the one thing that matters)?
- Default quiet-hours window, and where the user sets it (onboarding vs a settings view)?
