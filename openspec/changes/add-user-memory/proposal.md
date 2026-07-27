## Why

Nova's `SYSTEM_PROMPT` says "the user" — she has no idea who you are, and every session starts cold. That's fine for a fun show-off agent; it's the ceiling for a *personal* assistant. The single biggest gap between "impressive demo" and "the thing I open every morning" is that she must **know you** and **carry context across sessions**. This adds a per-user memory the bot injects at session start, plus tools so Nova grows and edits that memory in conversation. (Status: proposed.)

## What Changes

- **Server-side per-user memory store** (`pipecat-agent`, new `memory.py`): facts persisted per authenticated user (id from `secure-bot-endpoint`). One fact per entry — `{ text, category, created_at }`, category ∈ `preference | person | place | routine | project`. Starts as a JSON file per user, upgradeable to SQLite. Mirrors the one-fact-per-entry shape of the maintainer's own Claude memory index.
- **Context injection at session start** (`bot.py`): when `LLMContext` is built, load the user's profile + a token-budgeted set of relevant facts into a leading `system`/`developer` message, so Nova opens each session already knowing your name, home/work, the people she'll hear about, and your standing preferences.
- **Three memory tools** (`tools.py`): `remember(fact, category?)`, `recall(query)`, `forget(fact)` — Nova stores, retrieves, and edits mid-conversation ("remember I hate cilantro", "my mom's birthday is the 14th", "forget the old apartment").
- **Proactive-capture rule** in `SYSTEM_PROMPT`: salient, stable facts (names, preferences, routines) get remembered *without* being told to; anything sensitive (health, finances, relationships) is confirmed before it's stored.
- **Onboarding seed**: first run captures name, home/work, key people, and a few preferences so she's useful on day one — folded into the existing `app-onboarding-and-permissions` flow.

## Capabilities

### New Capabilities
- `user-memory`: a persistent, per-user, category-tagged fact store; injected into Nova's context each session and grown/edited via `remember`/`recall`/`forget`, with proactive capture and consent-gated storage of sensitive facts.
- `session-continuity`: a short rolling summary persisted per user and re-injected next session so follow-ups like "where'd we land on that?" resolve across the session boundary.

### Modified Capabilities
<!-- None — no prior OpenSpec specs exist for these; the onboarding seed is captured as an interaction with app-onboarding-and-permissions, not a spec change here. -->

## Impact

- **Agent (`pipecat-agent`):** new `memory.py` (store + retrieval + budgeted injection), `bot.py` (context build + prompt), `tools.py` (+3 tools & schemas, registered in `register_tool_handlers`).
- **Dependencies:** `secure-bot-endpoint` supplies the per-user identity that keys the store — without it there is no "user" to remember. `app-onboarding-and-permissions` supplies the day-one seed.
- **Interaction with other work:** `add-daily-assistant-tools` reuses this store for reminders/notes; `add-proactive-briefings` reads profile facts (home/work/commute) for leave-by timing.
- **Non-goals:** on-device-only memory (the bot builds context server-side, so it can't inject what it can't see — see design D1); a full visual memory-editing UI (voice edit via `forget`/`recall` only for now); cross-user or shared memory.
