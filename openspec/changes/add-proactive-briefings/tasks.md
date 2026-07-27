## 1. Proactive delivery path (foreground, ship first)

- [ ] 1.1 `bot.py`: inject a synthetic "proactive event" that triggers a Nova turn without a user utterance (speak + optionally paint a view)
- [ ] 1.2 Turn-safety: a proactive turn waits for a clear moment and never fires mid-user-utterance (reuse VAD/turn state)
- [ ] 1.3 On-connect briefing hook (Nova can greet with the digest when a session opens)

## 2. Scheduler

- [ ] 2.1 Per-user scheduler module on the droplet owning fire times for briefings, reminders, timers, and leave-by
- [ ] 2.2 Persist scheduled jobs so they survive bot restarts
- [ ] 2.3 Fire dispatch chooses sink by connection state: connected → proactive turn; not connected → push (§4)

## 3. Quiet hours + frequency caps

- [ ] 3.1 Per-user quiet-hours window (captured in onboarding) respected by all proactive delivery
- [ ] 3.2 Frequency caps (per hour/day); batch deferrable nudges into the next briefing
- [ ] 3.3 One-tap "stop reminding me about this" / global proactive on-off

## 4. Background push (APNs / FCM, phase 2)

- [ ] 4.1 App registers for push; hand the APNs/FCM token to the bot (keyed to user id)
- [ ] 4.2 Bot sends a tappable push when the user is not connected
- [ ] 4.3 Tapping a push opens the app to the relevant view (`realtime.rs` / `lib.rs` routing)

## 5. Briefings

- [ ] 5.1 Assemble the digest: `check_calendar` + `get_weather` + `list_reminders` + one headline
- [ ] 5.2 On-demand ("what's my day?") → spoken summary + `agenda`/digest view
- [ ] 5.3 Scheduled briefing at the user's set time (spoken if connected, else push)

## 6. Leave-by nudges (phase 3)

- [ ] 6.1 For a located calendar event, compute `event_start − travel_time − prep_buffer` via `get_directions`
- [ ] 6.2 Recompute near fire time for live traffic; phrase as "leave ~in N", not false precision
- [ ] 6.3 Deliver (spoken if connected, else push); resolve origin from profile home/work or live GPS

## 7. Reminder / timer fires

- [ ] 7.1 Due reminders/timers (from `add-daily-assistant-tools`) fire via the scheduler → proactive turn or push

## 8. Location triggers (optional, phase 4)

- [ ] 8.1 Arrive-home / leave-work triggers via `ios_location.rs`, under the same caps + DND

## 9. Verify

- [ ] 9.1 A scheduled morning briefing fires at the set time (spoken in-app when connected; push when closed)
- [ ] 9.2 A leave-by nudge arrives with enough lead time for a real event with a location
- [ ] 9.3 Nothing fires inside the quiet-hours window; caps hold under a burst
- [ ] 9.4 A proactive turn never interrupts an in-progress user utterance
