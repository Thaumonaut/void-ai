## 1. Reconnect + timeouts + truthful state (DONE)

- [x] 1.1 Reconnect supervisor with backoff; distinguish user-stop from per-attempt drop; give up after N never-connected attempts
- [x] 1.2 `connect_timeout` + `timeout` on the offer POST; check HTTP status before parsing the answer
- [x] 1.3 Time-boxed session teardown that aborts a hung `pc.close()`
- [x] 1.4 `ConnState` callback → `rt-live` from real peer state; reset `rt-connected` + `rt-failed` on terminal failure
- [x] 1.5 Surface `rt-status` in the strip (connecting/reconnecting/retry)

## 2. Remaining

- [ ] 2.1 Recover from the Ultravox ~8-min S2S session cap (proactively re-establish before/at the close event and replay context) — see `add-s2s-fullduplex-mode`
- [ ] 2.2 Gate reconnect on a network-reachability signal (don't spin retries while known-offline) — depends on `add-ui-states`
- [ ] 2.3 Verify the reconnect flow live on a real device (tap Start, drop network → reconnect; exhaust → retry prompt)
- [ ] 2.4 Tune backoff/attempt budget from real-world cellular behavior
