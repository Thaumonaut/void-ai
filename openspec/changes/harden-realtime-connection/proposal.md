## Why

The realtime voice session was tap-optimistic and terminal: `rt_connected` flipped on tap before the socket connected, the real status rendered nowhere, and a dropped WebRTC connection killed Nova permanently with the UI still showing "connected." On the target's spotty cellular this is guaranteed and indistinguishable from Nova being quiet. (Audit P0 — flagged by all three auditors.)

## What Changes

- **Auto-reconnect with backoff** (`realtime.rs`): a supervisor retries on unexpected drop; a real session that drops retries fast, repeated never-connected attempts back off and give up ("tap to retry"). *(Done.)*
- **Bounded connect + teardown** (`realtime.rs`): `connect_timeout(6s)` + `timeout(12s)` on the offer POST; time-boxed session teardown that aborts a hung `pc.close()`; HTTP status checked before parsing the answer. *(Done.)*
- **Truthful UI state**: a `ConnState` callback drives `rt-live` from the real peer state; terminal failure resets `rt-connected` (button/tap becomes retry) and shows a failure prompt; the strip surfaces the live status while connecting/reconnecting. *(Done.)*
- **Remaining:** recover from the Ultravox ~8-min S2S session cap (proactive re-establish); integrate a network-reachability signal (see `add-ui-states`); verify the reconnect flow live on a real device.

## Capabilities

### New Capabilities
- `realtime-connection-resilience`: the voice session survives transient network loss (auto-reconnect), never hangs indefinitely (timeouts), and the UI always reflects the true connection state.

## Impact

- `kaira-slint/src/realtime.rs` (supervisor + `run_attempt`, timeouts, `ConnState`), `src/lib.rs` (state callback, `rt-live`/`rt-failed`), `ui/app.slint` (status surfaced). Backend S2S-cap handling touches `pipecat-agent/bot.py`.
- Non-goals: server-side session/SPOF fixes (see `secure-bot-endpoint`).
