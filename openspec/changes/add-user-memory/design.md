## Context

Nova runs as a server-side Pipecat cascade; the client is deliberately keyless and all context is assembled in `bot.py` at `LLMContext` build time. That makes the bot the only place that can inject "who you are" into the model. Today it injects a static `SYSTEM_PROMPT` and nothing user-specific. This design adds a per-user memory the bot owns, loads, and grows.

## Goals / Non-Goals

**Goals:** Nova opens every session knowing the user; she remembers salient facts without being asked; memory survives reconnects and app restarts; per-user isolation.
**Non-Goals:** on-device memory; a visual editor; shared/multi-user memory; semantic search on day one.

## Decisions

**D1 — Server-side store, not on-device.** Context is built on the bot; the model can only be primed with what the bot can read. A device-local store would be more private but invisible to the context builder, defeating the point. Privacy is addressed by keying storage to the authenticated user (see D4) and colocating it with the already-trusted bot. *Alt:* device-local + upload-on-connect — more moving parts, larger data channel payloads, still lands on the server in context.

**D2 — One fact per entry, category-tagged, timestamped.** `{ text, category, created_at }` with category ∈ `preference|person|place|routine|project`. Simple to write, easy to budget, easy to prune. Retrieval starts as substring/keyword match over `text`+`category`; an embedding index is a later, additive upgrade. *Alt:* one big free-text profile blob — cheaper to inject but impossible to edit or budget precisely.

**D3 — Budgeted injection.** Never dump the whole store into context. Inject the always-on profile (name, home/work, top preferences) plus the top-N facts by recency/relevance under a fixed token budget, most-relevant first. Keeps first-token latency and cost bounded as the store grows.

**D4 — Identity comes from `secure-bot-endpoint`.** The store is keyed by the authenticated user id. Until auth lands, a single-user dev key (env) stands in. This is the hard dependency: "personal" memory is meaningless without a per-user key, and cross-user leakage is the one unacceptable failure.

**D5 — Proactive capture is prompt-driven, consent-gated for sensitive facts.** The `SYSTEM_PROMPT` instructs Nova to `remember` stable, salient facts on her own, but to confirm before storing anything sensitive (health, finances, relationships, precise home address). Storage is an action with consequences, so it follows the same "confirm before it matters" instinct as write actions elsewhere.

## Risks / Trade-offs

- **Over-remembering / clutter** → facts accrue faster than they're useful. Mitigate with categories, recency-weighted injection, and `forget`; consider a periodic dedupe/summarize pass.
- **Wrong or stale facts** → a confidently-wrong personal assistant is worse than a fun toy that's wrong. `forget`/correction must be trivially easy by voice; timestamps let newer facts win.
- **Privacy blast radius** → once she stores personal facts, the bot endpoint is holding personal data. Gate on `secure-bot-endpoint`; document where data lives (droplet) and that it is per-user isolated.
- **Latency creep** → unbounded injection slows every turn. D3's budget caps it.

## Migration Plan

Additive. Ship the store + injection + tools behind the existing bot; empty memory is a no-op (behaves like today). Seed via onboarding once available. JSON→SQLite is an internal swap behind `memory.py`. No client changes required for the core (tools + injection are server-side); a memory-review UI is a later, optional client add.

## Open Questions

- Encrypt at rest on the droplet, or rely on host + endpoint auth for now?
- When (if ever) does a periodic summarize/compaction pass run over an aging store?
- Do we expose a read-only "here's what I remember about you" view in the app for trust/transparency?
