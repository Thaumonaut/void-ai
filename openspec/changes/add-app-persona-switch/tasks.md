## 1. Assets

- [ ] 1.1 Import Kaira's frames from `Gmango/Aiko - Talking`: `Aiko-Idle.webp` (idle) + `Expanded Animation/Frame 1–11.webp` (talking loop, 540×675)
- [ ] 1.2 Convert to PNG under `kaira-slint/assets/kaira/` (`idle.png`, `talk/01.png … 11.png`); keep 540×675
- [ ] 1.3 (Optional) keep the 2-frame talking pair as a lighter fallback

## 2. Persona model (app side)

- [x] 2.1 A per-persona table for `nova`/`kaira`: `Persona` global in `app.slint` (index, id, name, name-upper, accent) + `bot_url_for(index)` in `lib.rs`
- [x] 2.2 Selected-persona state in the app (`Persona.index`, default `nova`)  <!-- persistence across launches deferred -->
- [x] 2.3 Wire `bot_url` selection into `lib.rs` so connect uses the selected persona's URL

## 3. Swipe gesture

- [x] 3.1 Horizontal-drag detection on the agent-icon `TouchArea` (`pointer-event` + 44px threshold; falls through to tap otherwise)
- [x] 3.2 Swipe toggles the selected persona (Nova ⇄ Kaira)
- [x] 3.3 On persona change while connected: reconnect to the new persona's `bot_url` (`on_persona_switched` in `lib.rs`)

## 4. Per-persona identity

- [ ] 4.1 Agent-icon component switches on `icon_kind` (orb for Nova, avatar for Kaira)
- [x] 4.2 Per-persona name in the agent label + a persona-tinted accent glow behind the orb (violet=Nova, green=Kaira)
- [x] 4.3 A "SWIPE TO SWITCH" hint on the connect label  <!-- richer transition/affordance can follow -->
- [ ] 4.1 Agent-icon switches on `icon_kind` (orb vs avatar) — Phase 2 (Kaira avatar); Phase 1 uses the orb for both

## 5. Kaira avatar animation

- [ ] 5.1 Preload the talking frames into an image array (Rust → Slint model)
- [ ] 5.2 Idle: show `idle.png`; Speaking: cycle talking frames (~8–12 fps) while the agent-speaking flag is set
- [ ] 5.3 Drive the speaking flag from the existing `AGENT_PLAYING` state (`realtime.rs`/`lib.rs`)

## 6. Two-bot wiring (deploy)

- [ ] 6.1 Run Nova and Kaira as two instances (`KAIRA_PERSONA=nova` / `kaira`) on separate ports — update `pipecat-agent` deploy + `kaira-slint/servers.sh`
- [ ] 6.2 Configure the app's two `bot_url`s (Nova, Kaira)

## 7. Verify

- [ ] 7.1 Swipe on the agent icon toggles Nova ⇄ Kaira (name + icon + palette change)
- [ ] 7.2 After a swipe, the app is talking to the *correct* persona's bot (Kaira greets as a GP in the Sulafat voice; Nova is sassy)
- [ ] 7.3 Kaira's avatar sits on the idle pose when quiet and animates the talking loop while she speaks
- [ ] 7.4 Tap (connect/disconnect) and long-press behaviors still work — no gesture collision
