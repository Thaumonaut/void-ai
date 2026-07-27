## 1. Store

- [ ] 1.1 `pipecat-agent/memory.py`: per-user store keyed by user id — `add(fact, category)`, `query(text) -> [fact]`, `remove(match)`, `profile(user)`; JSON-file backend with a clean interface for a later SQLite swap
- [ ] 1.2 Fact shape `{ text, category, created_at }`, category ∈ `preference|person|place|routine|project`; timestamps so newer facts win on conflict
- [ ] 1.3 Single-user dev key fallback (env) until `secure-bot-endpoint` supplies real per-user ids

## 2. Injection

- [ ] 2.1 `bot.py`: at `LLMContext` build, load `profile(user)` + top-N relevant facts and prepend as a `system`/`developer` message
- [ ] 2.2 Token-budgeted selection (profile always on; remaining facts by recency/relevance, most-relevant first, hard cap)
- [ ] 2.3 Empty-memory path is a no-op (identical to today's behavior)

## 3. Tools

- [ ] 3.1 `tools.py`: `remember(fact, category?)`, `recall(query)`, `forget(fact)` + schemas in `NOVA_TOOLS`; register in `register_tool_handlers`
- [ ] 3.2 `recall` returns a short summary Nova reacts to (react-don't-recite, consistent with existing tools)
- [ ] 3.3 `forget` matches leniently (substring/nearest) and confirms what was removed

## 4. Prompt

- [ ] 4.1 `SYSTEM_PROMPT`: proactively `remember` stable/salient facts without being asked
- [ ] 4.2 `SYSTEM_PROMPT`: confirm before storing sensitive facts (health, finances, relationships, precise address)

## 5. Session continuity

- [ ] 5.1 Persist a short rolling summary per user at session end (or on disconnect)
- [ ] 5.2 Re-inject the summary at next session start so "where'd we land?" resolves
- [ ] 5.3 Cap summary size; compress/overwrite oldest rather than growing unbounded

## 6. Onboarding seed

- [ ] 6.1 Capture name, home/work, key people, a few preferences during `app-onboarding-and-permissions`
- [ ] 6.2 Write the seed into the store as first-class facts (so day-one Nova is already personalized)

## 7. Verify

- [ ] 7.1 A fact stored in one session is present in Nova's context in a later, fresh session
- [ ] 7.2 `forget` removes a fact and it no longer appears in context
- [ ] 7.3 Sensitive-fact path asks for confirmation before storing
- [ ] 7.4 Two distinct user ids never see each other's facts
