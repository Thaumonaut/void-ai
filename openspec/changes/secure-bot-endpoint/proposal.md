## Why

The Pipecat bot is an open, unauthenticated endpoint on a public IP (`143.198.134.89:8080`) that is compiled into every shipped client. Anyone who learns the IP can open sessions and **burn the project's paid Soniox / OpenRouter / SerpApi / Mapbox / Tavily quota**, or hold the single session slot. Signaling is plaintext HTTP (MITM-able) with App Transport Security disabled app-wide, API keys can leak into `docker logs` (secrets in request URLs), `/ice` vends 24h TURN creds unauthenticated, and user GPS is logged as PII. (Audit P0/P1.)

## What Changes

- **Authenticate `/api/offer`** (shared secret / signed offer) + per-IP **rate limiting**; front the raw runner with a reverse proxy.
- **TLS**: put the bot behind a domain with Caddy auto-TLS; replace blanket `NSAllowsArbitraryLoads` with a scoped exception for just the bot host.
- **Stop leaking secrets to logs**: move provider keys out of request URLs (into headers where supported) and sanitize before `logger.exception`/tool-result strings; coarsen/drop the GPS log line.
- **Scope tokens**: confirm/rotate the Mapbox `pk.` token restricted to bundle/URL + minimal scopes; consider proxying Directions through the bot.
- **Lock down `/ice`**: short TTL + same auth as `/api/offer`.
- **Resolve the URL by DNS** so the bot can be repointed without an app rebuild.

## Capabilities

### New Capabilities
- `bot-endpoint-security`: the bot rejects unauthenticated/over-rate callers, serves TLS, and does not leak secrets/PII into logs — so its paid quota and user data are not exposed to anyone who learns the IP.

## Impact

- `pipecat-agent/bot.py` (auth middleware, rate limit, `/ice`, logging sanitization), `deploy/` (Caddy + domain + DNS), `kaira-slint/project.yml` (scoped ATS), `kaira-slint/src/lib.rs` (bot URL by domain), `ios_map.rs` (token). SPOF/multi-session scale is a related but separate concern (per-session workers / failover).
- Non-goals: the client-side reconnect (see `harden-realtime-connection`); a full multi-tenant scale-out.
