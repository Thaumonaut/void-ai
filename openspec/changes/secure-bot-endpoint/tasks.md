## 1. Auth + rate limit

- [ ] 1.1 Require a shared secret / signed offer on `/api/offer`; reject unauthenticated callers before any tool/LLM spend
- [ ] 1.2 Per-IP (or per-token) rate limiting
- [ ] 1.3 Apply the same auth + a short TTL to `/ice`

## 2. TLS + transport security

- [ ] 2.1 Put the bot behind a domain with Caddy auto-TLS
- [ ] 2.2 Point the client at `https://<domain>/api/offer` (via DNS, repointable)
- [ ] 2.3 Replace `NSAllowsArbitraryLoads` with a scoped `NSExceptionDomains` entry for the bot host

## 3. Secrets + PII hygiene

- [ ] 3.1 Move provider keys out of request URLs into headers where supported (Mapbox/SerpApi)
- [ ] 3.2 Sanitize errors before `logger.exception` / tool-result strings — provider + status only, never the key/URL
- [ ] 3.3 Drop/coarsen the GPS log line
- [ ] 3.4 Confirm the Mapbox `pk.` token is bundle/URL-restricted + minimally scoped; rotate; consider proxying Directions via the bot

## 4. Resilience posture (related)

- [ ] 4.1 Document/plan a move off single-droplet single-session (per-session workers / warm failover) — out of scope to fully build here, but capture the plan
- [x] 4.2 Idle-session reaper — cancel a session whose inbound media (`InputAudioRawFrame`) stopped for `KAIRA_IDLE_TIMEOUT_SECS` (default 60s), so a dropped/backgrounded client stops burning STT/LLM/TTS quota. Uses pipecat's idle detection but on media (not Bot/UserSpeaking) frames, and cancels only the session (`cancel_runner_on_idle_timeout=False`) so the server stays up. Server-side counterpart to the client reconnect supervisor (`harden-realtime-connection`). Verified with a headless idle-reap test (no false reap while media flows; reaps on stop)
- [x] 4.3 Max-session hard cap (`KAIRA_MAX_SESSION_SECS`, default 30 min) — backstop that ends even an always-active session
- [ ] 4.4 Verify reaping live on the droplet (drop network mid-call → session reaped in logs; reconnect → fresh session)
