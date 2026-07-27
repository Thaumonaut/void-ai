## 1. Foundation — RTVI UI-control contract (DONE)

- [x] 1.1 Define the UI-control message contract (`kaira-slint/UI_CONTRACT.md`): ops open_view/close_view/focus_view/collapse/fullscreen + content images/map/products/web/doc
- [x] 1.2 Bot send path: `ui.py` `UiBridge` (urgent transport messages on the RTVI `chat` channel)
- [x] 1.3 Client recv path: `realtime.rs` `UiControlCb` + `server-message` case → callback

## 2. View surface (DONE)

- [x] 2.1 Slint `VS` global + view surface (chat/images/products/map/web/doc, switcher rail, collapse/fullscreen)
- [x] 2.2 `apply_ui_control` parses ops → sets `VS` props/models
- [x] 2.3 Light + dark, safe-area insets, line icons, Nova constellation, emulator-verified

## 3. Tool layer + prompt/filler (DONE)

- [x] 3.1 6 tools in `tools.py`: search_places, get_directions, search_web, search_images, search_products, start_navigation
- [x] 3.2 Filler banter Layer 1 (curated in-character line per tool, spoken on call)
- [x] 3.3 SYSTEM_PROMPT (Nova persona, screen-aware, cover-the-wait, react-don't-recite)
- [x] 3.4 Verify tool-calling reliability on the cascade (Gemma-4/Cerebras: 10/10 calls, 0/4 over-calls)
- [x] 3.5 Live end-to-end over WebRTC

## 4. Remaining tail

- [ ] 4.1 Client→server view events (user-driven view changes reported back to the bot; `on_app_message` handler)
- [ ] 4.2 Reconcile web/doc rendering vs. the interactive-Mapbox WebView — decide whether web/doc move to a WebView or stay block-based
- [ ] 4.3 Real remote thumbnails (replace placeholder tints) + in-chat image thumbnails
- [ ] 4.4 Deferred UI polish: 4-side movable switcher, full-screen Nova pause/mute dock, mini-constellation in the collapsed strip
- [ ] 4.5 Filler Layer 2 (LLM one-liner emitted with the call; suppress Layer 1 when present) — only if a cooperating model is adopted
- [ ] 4.6 Gate the debug auto-seed demo content off for release builds
