## Why

Nova's six tools (`search_places`, `search_web`, `search_images`, `search_products`, `get_directions`, `start_navigation`) are all outward-facing lookups — "show me / take me". They make a great demo, but daily *habit* comes from manage-my-life tasks: what's on my calendar, remind me to X, set a timer, what's the weather. This adds the habit-forming core so Nova earns a daily open, and — because these are her first tools that *write* to your accounts — it establishes a confirm-before-write rule so the sass never yolos a destructive action. (Status: proposed.)

## What Changes

- **New tools** (`pipecat-agent/tools.py`, registered in `bot.py`):
  - `check_calendar(range)`, `create_event(title, when, where?)`, `move_event(which, to)` — Google Calendar (per-user OAuth).
  - `add_reminder(text, when?)`, `list_reminders()`, `complete_reminder(which)` — persisted server-side (reuses the `add-user-memory` store).
  - `set_timer(duration, label?)`, `list_timers()` — server-side scheduled jobs (fire via `add-proactive-briefings`' push path).
  - `get_weather(when?, place?)` — Open-Meteo (**keyless**, no new secret; fits the keyless-client ethos).
  - `add_note(text)`, `list_notes()` — quick capture, persisted server-side.
  - *(optional, phased)* `play_music(query)`, `pause_music()` — Spotify (per-user OAuth).
- **Two new views** (`kaira-slint`): `agenda` (a day/calendar view) and `list` (reminders / tasks / notes). Add the ops to `UI_CONTRACT.md`; render them in `realtime.rs` → `lib.rs` (`VS` models) → `ui/app.slint`. Block-based like the existing `web`/`doc` views (fast on 3G), not WebView.
- **Confirm-before-write rule**: reading is free; anything that creates/moves/deletes on your calendar, sends a message, or spends money — Nova states what she's about to do and waits for a yes. Enforced in `SYSTEM_PROMPT` and structurally in each write tool.

## Capabilities

### New Capabilities
- `assistant-daily-tools`: the calendar / reminders / timers / weather / notes tool set and the `agenda` + `list` views they render into.
- `write-action-confirmation`: the cross-cutting rule that Nova confirms before any outbound, destructive, or spending action, while reads are never blocked.

### Modified Capabilities
<!-- None — extends the tool + view-surface pattern established by nova-agent-tooling without changing its specs. -->

## Impact

- **Agent (`pipecat-agent`):** `tools.py` (new tools + schemas), `bot.py` (registration + prompt rule), `ui.py` (`UiBridge.agenda` / `UiBridge.list`). New provider integrations: Google Calendar (OAuth), Open-Meteo (keyless), Spotify (OAuth, phased).
- **App (`kaira-slint`):** `UI_CONTRACT.md` (+`agenda`, +`list` ops), `realtime.rs` (parse ops), `lib.rs` (`VS` models), `ui/app.slint` (two views + switcher entries).
- **Dependencies:** `add-user-memory` — its per-user store holds reminders/notes and its identity keys per-user calendar/Spotify OAuth tokens. `secure-bot-endpoint` — user id + OAuth token custody.
- **Interaction with other work:** `add-proactive-briefings` fires the reminders/timers created here and reads the calendar for leave-by nudges; `add-turn-by-turn-navigation`'s `get_directions` is reused by those nudges for ETA.
- **Non-goals:** email triage, "look at this" vision/camera Q&A, and personal-document Q&A (a future `add-assistant-comms-and-vision` change); multi-provider task sync (e.g., ClickUp) beyond the built-in reminder store.
