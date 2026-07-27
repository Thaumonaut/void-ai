## 1. Weather (keyless, ship first)

- [ ] 1.1 `get_weather(when?, place?)` in `tools.py` via Open-Meteo (geocode place → forecast); no new secret
- [ ] 1.2 Return a short summary Nova reacts to; surface a compact forecast in a view (agenda header or a small card)

## 2. Views: agenda + list

- [ ] 2.1 `UI_CONTRACT.md`: add `agenda` (day/calendar) and `list` (reminders/tasks/notes) ops with payload shapes
- [ ] 2.2 `ui.py`: `UiBridge.agenda(...)` and `UiBridge.list(...)`
- [ ] 2.3 `realtime.rs`: parse the two new ops → `UiControlCb`
- [ ] 2.4 `lib.rs`: `VS` models for agenda + list; `app.slint`: render both, block-based, light+dark, safe-area, switcher entries

## 3. Calendar (Google)

- [ ] 3.1 Per-user OAuth token custody server-side (keyed by user id; depends on `secure-bot-endpoint`)
- [ ] 3.2 `check_calendar(range)` (read) → populate the `agenda` view
- [ ] 3.3 `create_event(title, when, where?)` and `move_event(which, to)` (writes) → gated by `write-action-confirmation`

## 4. Reminders / tasks

- [ ] 4.1 `add_reminder(text, when?)`, `list_reminders()`, `complete_reminder(which)` persisted in the `add-user-memory` store
- [ ] 4.2 A dated reminder registers a scheduled job for `add-proactive-briefings` to fire
- [ ] 4.3 Render reminders/tasks in the `list` view

## 5. Timers

- [ ] 5.1 `set_timer(duration, label?)`, `list_timers()` as bot-owned scheduled jobs
- [ ] 5.2 Firing is handled by `add-proactive-briefings` (this change only creates/lists)

## 6. Notes

- [ ] 6.1 `add_note(text)`, `list_notes()` persisted in the store; render in the `list` view

## 7. Write-action confirmation

- [ ] 7.1 `SYSTEM_PROMPT`: confirm before any calendar write, message send, or spend; never confirm on reads
- [ ] 7.2 Structural guard in write tools (require a confirmed intent) so a model slip can't silently commit
- [ ] 7.3 Confirmation copy stays in-character but states the exact action unambiguously

## 8. Music (optional, phased)

- [ ] 8.1 `play_music(query)`, `pause_music()` via Spotify per-user OAuth

## 9. Verify

- [ ] 9.1 "What's my day?" reads calendar and fills the agenda view without confirmation
- [ ] 9.2 "Move my 3pm to tomorrow" states the change and waits for a yes before writing
- [ ] 9.3 "Remind me to call the vet at 4" persists and later fires (with `add-proactive-briefings`)
- [ ] 9.4 "Set a 10-minute timer" fires even with the app backgrounded
- [ ] 9.5 "What's the weather?" answers with no key configured
