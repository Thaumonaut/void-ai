## 1. Tool-in-progress ("Nova is working")

- [ ] 1.1 Add a `working` op to the UI-control contract (view + optional label), fired by the bot before a slow tool
- [ ] 1.2 Render a skeleton/shimmer or status pill in the target view on `working`; clear it when content lands

## 2. Error / failed-fetch states

- [x] 2.1 Track a per-slot `failed` flag; set it on fetch failure so images/products show "couldn't load" instead of a permanent placeholder — verified on sim
- [ ] 2.2 Tap-to-retry (re-fetch) for failed image/product thumbnails
- [ ] 2.3 "Map unavailable" affordance when the map tile/webview can't load
- [x] 2.4 Fix stale-async overwrite (audit #9): per-collection generation counter — bump on model replace, capture at fetch spawn, drop stale landings. Verified on sim (before: phantom photo; after: "couldn't load")

## 3. Empty states

- [x] 3.1 Per-view empty state (reusable `EmptyState` = icon + hint) for images/products/web/docs — verified on sim
- [x] 3.2 Suppress `FOUND 0` / `0 stores` headers when the list is empty (content branch gated on `length > 0`)

## 4. Offline detection

- [ ] 4.1 Platform reachability listener (iOS + Android) → a VS `offline` property
- [ ] 4.2 Global offline banner; connect action reflects offline
- [ ] 4.3 Feed the reachability signal into reconnect gating (`harden-realtime-connection` 2.2)
