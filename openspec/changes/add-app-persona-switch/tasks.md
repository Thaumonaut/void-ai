<!-- SHIPPED 2026-07-27: Phases 1–3 live — deployed to the droplet (Nova :8080 / Kaira :8081) and
     installed on-device. Remaining unchecked items are optional polish / a follow-up. -->

## 1. Assets

- [x] 1.1 Import Kaira's frames from `Gmango/Aiko - Talking`: `Aiko-Idle.webp` (idle) + `Expanded Animation/Frame 1–11.webp` (talking loop)
- [x] 1.2 Convert to PNG under `kaira-slint/assets/kaira/` (`idle.png`, `talk/01.png … 11.png`)
- [ ] 1.3 (Optional) keep the 2-frame talking pair as a lighter fallback

## 2. Persona model (app side)

- [x] 2.1 A per-persona table for `nova`/`kaira`: `Persona` global in `app.slint` (index, id, name, name-upper, accent) + `bot_url_for(index)` in `lib.rs`
- [x] 2.2 Selected-persona state in the app (`Persona.index`, default `nova`)  <!-- persistence across launches deferred -->
- [x] 2.3 Wire `bot_url` selection into `lib.rs` so connect uses the selected persona's URL

## 3. Swipe gesture

- [x] 3.1 Horizontal-drag detection on the agent-icon `TouchArea` (`pointer-event` + 44px threshold; tap otherwise)
- [x] 3.2 Swipe toggles the selected persona (Nova ⇄ Kaira); a Settings → Agent toggle does too
- [x] 3.3 On persona change while connected: reconnect to the new persona's `bot_url` (`on_persona_switched` in `lib.rs`)

## 4. Per-persona identity

- [x] 4.1 Agent icon switches on persona: Nova = galaxy orb, Kaira = the doctor avatar (`Persona.index` ternary on the `Image` source)
- [x] 4.2 Per-persona name in the agent label + accent; the whole app recolors (VS.orange/slab-* → green for Kaira, coral for Nova)
- [x] 4.3 A "SWIPE TO SWITCH" hint + a one-shot swap animation (glow pulse + name-flash pill)  <!-- richer transitions can follow -->

## 5. Kaira avatar animation

- [x] 5.1 Preload the 11 talking frames into a `[image]` array in `app.slint`
- [x] 5.2 Idle: show `idle.png`; Speaking: cycle talking frames (~11 fps) while the agent-speaking flag is set
- [x] 5.3 Drive the speaking flag from the inbound level (debounced 500ms hold — also fixes the "TALKING⇄TAP TO STOP" flicker)

## 6. Two-bot wiring (deploy)

- [x] 6.1 Nova + Kaira as two droplet instances (`KAIRA_PERSONA=nova`/`kaira`) on `:8080`/`:8081` (`--network host`, `--restart unless-stopped`)
- [x] 6.2 App's two `bot_url`s wired (`bot_url_for`: droplet `:8080` Nova / `:8081` Kaira; local override via `realtime_url*.txt`)

## 7. Verify

- [x] 7.1 Swipe toggles Nova ⇄ Kaira (name + icon + palette change) — verified on the iOS sim
- [x] 7.2 After a swipe, the app talks to the *correct* persona's bot (Kaira greets as a GP in Sulafat; Nova is sassy) — verified via droplet logs
- [x] 7.3 Kaira's avatar sits on the idle pose when quiet and animates the talking loop while she speaks — verified
- [x] 7.4 Tap (connect/disconnect) still works — no gesture collision (tap-vs-swipe fully in `pointer-event`)
