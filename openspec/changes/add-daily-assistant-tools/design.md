## Context

Nova drives a Slint view surface from server-side tools over the RTVI channel (the `nova-agent-tooling` pattern: a tool fetches, pushes to a view via `UiBridge`, and returns a short summary). This change extends that pattern from outward lookups to personal, stateful, sometimes-writing tools — which introduces two new concerns the lookup tools never had: per-user auth to third-party accounts, and actions with side effects.

## Goals / Non-Goals

**Goals:** the four habit-forming tools (calendar, reminders, weather, timers) plus notes; two clean views (`agenda`, `list`); a safety rule so writes are never silent.
**Non-Goals:** email/vision/doc-Q&A (separate change); external task-manager sync; a settings UI for provider auth beyond onboarding.

## Decisions

**D1 — Keyless where possible; OAuth only where required.** Weather uses Open-Meteo, which needs no key — no new secret, consistent with the keyless-client ethos. Calendar and Spotify inherently need per-user OAuth; their tokens live server-side under the user's id (leaning on `add-user-memory` + `secure-bot-endpoint`). *Alt:* a keyed weather provider — rejected, no benefit over Open-Meteo for this use.

**D2 — Reminders / notes / timers are server-side state, not device-local.** They must survive reconnects and, crucially, fire when the app is closed (via `add-proactive-briefings`' push path). Device-local timers die when the app is killed. Reuse the `add-user-memory` store rather than standing up a second persistence layer.

**D3 — Timers/reminders are scheduled jobs owned by the bot.** `set_timer`/`add_reminder` register a job with a fire time; delivery is the proactive path's job. This change *creates and lists* them; `add-proactive-briefings` *fires* them. Clean seam, no duplicated scheduler.

**D4 — `agenda` and `list` are block-based, not WebView.** A day's events and a task list render instantly from small payloads; they don't need pan/zoom. Only the map earned a WebView. Keeps 3G payloads tiny and matches `web`/`doc`.

**D5 — Write-confirmation is behavioral *and* structural.** The prompt tells Nova to confirm before writes; the write tools also take/expect a confirmed intent so a model slip can't silently create/delete. Reads (`check_calendar`, `get_weather`, `list_*`) never confirm — over-confirming is its own failure mode that gets an assistant muted.

**D6 — Distinguish "plan" from "commit," matching the existing nav split.** `get_directions` (plan) vs `start_navigation` (commit) already models this. Calendar mirrors it: `check_calendar` is free; `create_event`/`move_event` commit and therefore confirm.

## Risks / Trade-offs

- **Over-confirming annoyance** vs **under-confirming danger** → only outbound/destructive/spending actions confirm; everything readable is instant.
- **OAuth token custody** → per-user tokens are sensitive; they ride on the same trust boundary as memory (`secure-bot-endpoint`). Don't ship calendar writes before that boundary exists.
- **Timer reliability with the app backgrounded (iOS)** → a server-owned job + push is the only reliable path; a purely in-app timer can't be trusted. This is why D2/D3 push scheduling to the bot.
- **Provider variance** (calendar recurrence, all-day events, time zones) → start with the common cases (today/this-week reads, single timed events); document the edges.

## Migration Plan

Ship read-only tools first (`check_calendar`, `get_weather`, `list_*`) — no auth-to-write risk, immediately useful. Add the `agenda`/`list` views alongside. Layer in writes (`create_event`, `move_event`, reminder/timer creation) once `write-action-confirmation` and `secure-bot-endpoint` are in place. Spotify is an optional last phase.

## Open Questions

- Built-in reminder store vs syncing to an external task manager (ClickUp) — start built-in; revisit if the user wants their existing lists.
- How much of the agenda Nova reads aloud vs shows (react-don't-recite says: show it, mention the one thing that matters).
- Time-zone / recurrence handling depth for v1.
