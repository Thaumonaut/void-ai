## Why

Nova is 100% reactive — she only speaks when summoned. A real assistant reaches out: a morning briefing, a "you should leave in 10 to make your 3pm", a reminder that actually fires. This adds the proactive delivery path Nova currently lacks and two flagship behaviors on top of it. The leave-by nudge is where the existing nav work pays off twice — it's calendar + `get_directions` ETA + a timely push, which no fun-agent demo does. (Status: proposed.)

## What Changes

- **Proactive delivery path** (`pipecat-agent`):
  - *Foreground / connected:* the bot kicks an **unprompted Nova turn** — a scheduled "proactive event" injected into the pipeline so she speaks and updates a view without a user utterance (today the LLM only runs on a user turn after STT).
  - *Background / app closed:* delivery via **push (APNs on iOS / FCM on Android)** — a tappable summary that opens the app to the briefing.
  - A **per-user scheduler** on the droplet owns fire times for briefings, reminders, and timers.
- **Briefings**: a morning/evening digest = calendar + weather + top reminders + one headline, in Nova's voice — on demand ("what's my day?") and on a user-set schedule.
- **Nudges**:
  - *Leave-by*: for a calendar event with a location, schedule a nudge at `event_start − travel_time − prep_buffer`, using `get_directions` for a live ETA; push "leave in 10."
  - *Reminder / timer fires*: due reminders and timers (from `add-daily-assistant-tools`) speak if connected, else push.
  - *(optional)* location triggers via `ios_location.rs` (arrive home / leave work).
- **Quiet hours + frequency caps**: a proactive assistant that over-nudges gets muted or deleted — so DND windows are respected and nudges are capped/batched into the briefing when possible.

## Capabilities

### New Capabilities
- `proactive-briefings`: on-demand and scheduled digests (calendar + weather + reminders + a headline) delivered via a spoken turn when connected or a push when not, respecting quiet hours.
- `proactive-nudges`: time- and context-triggered interruptions — leave-by, reminder/timer fires, optional location triggers — under frequency caps and DND, delivered over the same path.

### Modified Capabilities
<!-- None — this adds the proactive delivery path; it consumes the tools and views from add-daily-assistant-tools without changing their specs. -->

## Impact

- **Agent (`pipecat-agent`):** new scheduler module; `bot.py` (inject a proactive turn; on-connect briefing); `ui.py` (briefing → `agenda`/digest view). Reuses `check_calendar` / `get_weather` / `list_reminders` and `get_directions`.
- **App (`kaira-slint`):** register for push and hand the APNs/FCM token to the bot; route a tapped push to open the relevant view (`realtime.rs` / `lib.rs`); a quiet-hours setting (captured in onboarding).
- **Dependencies:** `add-daily-assistant-tools` (calendar/weather/reminders/timers to brief and fire), `add-user-memory` (per-user identity + profile facts like home/work/commute for leave-by), `add-turn-by-turn-navigation` (`get_directions` for ETA), `secure-bot-endpoint` (user id + push-token custody).
- **Non-goals:** multi-user broadcast; smart-home/automation; a full notification-preferences UI beyond quiet hours + a global on/off; keeping a live WebRTC voice session alive in the background (iOS won't — background is push-only, see design).
