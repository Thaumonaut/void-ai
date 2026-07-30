## 1. Images tabbed history (in-session)

- [x] 1.1 `IMAGES_TABS` history + `IMAGES_ACTIVE` (mirror `WEB_TABS`/`WEB_ACTIVE`); a batch = {query, items}
- [x] 1.2 The `images` op APPENDS a batch (+ sets it active) instead of replacing `VS.images`
- [x] 1.3 `sync_images(ui)` pushes count/titles/active-index + the active batch → `VS.images`
- [x] 1.4 Image tab strip in the UI (horizontal query chips; active chip highlighted)
- [x] 1.5 `on_switch_image_tab(i)` switches the active batch; history capped at `IMAGE_TAB_LIMIT` (12)

## 2. Web real reader

- [x] 2.1 Each Web tab loads its `url` in a WKWebView (`ios_web.rs`, mirrors `ios_map`) — real page, scrollable
- [x] 2.2 Nova's summary blocks become a "Nova's take" toggle in the toolbar (tucks the webview to reveal them)
- [~] 2.3 Tab strip switches pages (tab-switcher overlay); `go_back` exists in `ios_web` but no back button wired yet
- [x] 2.4 Android/desktop fallback: `has-web-reader=false` → the summary-block reader shows (no native webview)

## 3. Chat → tab jump

- [x] 3.1 Nova's image/web results drop a compact card in the chat timeline (icon + title + subtitle), woven in by anchor
- [x] 3.2 Tapping a chat card (`chat-jump`) switches to that view + selects the corresponding tab (stable id → current index)

## 4. Cross-session persistence (capped)

- [x] 4.1 Serialize batches (images: query+items; web: url+title+blocks) to `<files_dir>/view_history.json` (mobile only)
- [x] 4.2 Restore on launch into the tab history (`persist::load`); `sync_images` re-fetches thumbnails lazily; opens the views in the nav
- [x] 4.3 Cap: images 12 (in-session + stored), web stored cap 16; oldest evicted. Transcript itself not persisted (searches only)

## 5. Video search + view + vision (a grid ⨯ a player)

- [x] 5.1 Bot: `search_videos(query)` (SerpApi `engine=youtube`) → `ui.videos` op {title, channel, dur, url, id, thumb}; registered + in NOVA_TOOLS
- [x] 5.2 New Video view: results grid (thumb + play glyph + duration badge + title/channel) with tabbed history like Images
- [x] 5.3 Playback: tap a result → `ios_video` WKWebView (YouTube embed, inline autoplay); back/open-in-YouTube; grid ⇄ player
- [x] 5.4 Video vision (duplex): a 3s timer calls `takeSnapshot` → JPEG → `set_viewing_bytes` (reuses image-vision path)
- [~] 5.5 (Stretch) True video understanding — NOT done; frame-snapshot is the vision path for now
