## 1. Store

- [x] 1.1 `pipecat-agent/memory.py`: per-user store keyed by user id — `add`, `query(text) -> [fact]`, `remove(match)`, `all_facts`, `context_block` (profile/injection); JSON-file backend (atomic write-then-rename) behind a clean interface for a later SQLite swap
- [x] 1.2 Fact shape `{ text, category, created_at }`, category ∈ `preference|person|place|routine|project`; timestamps + dedupe so newer facts win on conflict
- [x] 1.3 Single-user dev key fallback (`MEMORY_USER_ID`, default `default`) + `?uid=` capture wired for when `secure-bot-endpoint` supplies real per-user ids

## 2. Injection

- [x] 2.1 `bot.py`: at `LLMContext` build, `memory.context_block(user_id)` is prepended to the system prompt (one system message → provider-safe across cascade / Gemini / Ultravox)
- [x] 2.2 Budgeted selection (identity-anchoring categories first, then recency; hard `budget_chars` cap)
- [x] 2.3 Empty-memory path is a no-op — `context_block` returns `""`, so the prompt is byte-for-byte today's

## 3. Tools

- [x] 3.1 `tools.py`: `remember(fact, category?)`, `recall(query)`, `forget(fact)` + schemas in `NOVA_TOOLS`; registered in `register_tool_handlers` (user_id closure)
- [x] 3.2 `recall` returns a short summary Nova reacts to (react-don't-recite); no filler (local + instant)
- [x] 3.3 `forget` matches leniently (substring both directions) and confirms what was removed

## 4. Prompt

- [x] 4.1 `SYSTEM_PROMPT`: proactively `remember` stable/salient facts without being asked
- [x] 4.2 `SYSTEM_PROMPT`: confirm before storing sensitive facts (health, finances, relationships, precise address)

## 5. Session continuity

- [x] 5.1 Persist a short rolling summary per user at disconnect (turn-tail recap → `memory.set_summary`)
- [x] 5.2 Re-inject the summary at next session start (via `context_block` "Where you left off last session")
- [x] 5.3 Cap summary size (`max_chars`, overwrites — never grows unbounded)

## 6. Onboarding seed

- [ ] 6.1 Capture name, home/work, key people, a few preferences during `app-onboarding-and-permissions` — DEFERRED: the app is voice-first with no text-entry UI; Nova already personalizes day-one via proactive `remember` in the first conversation. Revisit if we add a text seed form.
- [ ] 6.2 Write the seed into the store as first-class facts — depends on 6.1

## 7. Verify

- [x] 7.1 A fact stored in one session is present in a later, fresh session — unit + handler tests pass
- [x] 7.2 `forget` removes a fact and it no longer appears in context — verified
- [ ] 7.3 Sensitive-fact path asks for confirmation before storing — prompt-driven (4.2 done); needs a LIVE bot session to confirm the model behavior
- [x] 7.4 Two distinct user ids never see each other's facts — isolation test passes
