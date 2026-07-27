## 1. Tool-in-progress ("Nova is working")

- [ ] 1.1 Add a `working` op to the UI-control contract (view + optional label), fired by the bot before a slow tool
- [ ] 1.2 Render a skeleton/shimmer or status pill in the target view on `working`; clear it when content lands

## 2. Error / failed-fetch states

- [ ] 2.1 Track per-slot load state (loading/loaded/failed) in `spawn_image_fetch`
- [ ] 2.2 Broken-image glyph + tap-to-retry for images and product thumbnails
- [ ] 2.3 "Map unavailable" affordance when the map tile/webview can't load

## 3. Empty states

- [x] 3.1 Per-view empty state (reusable `EmptyState` = icon + hint) for images/products/web/docs — verified on sim
- [x] 3.2 Suppress `FOUND 0` / `0 stores` headers when the list is empty (content branch gated on `length > 0`)

## 4. Offline detection

- [ ] 4.1 Platform reachability listener (iOS + Android) → a VS `offline` property
- [ ] 4.2 Global offline banner; connect action reflects offline
- [ ] 4.3 Feed the reachability signal into reconnect gating (`harden-realtime-connection` 2.2)
