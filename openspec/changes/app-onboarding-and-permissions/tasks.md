## 1. Permission plumbing

- [x] 1.1 Request microphone permission in-app (iOS `AVAudioSession.requestRecordPermission` / Android runtime permission) — remove the reliance on `adb grant` — `src/permissions.rs` (`request_mic`), wired via `on_request_permissions`
- [x] 1.2 Request location permission in-app with rationale — `permissions::request_location` (iOS reuses `ios_location::start`; Android `requestPermissions`), primed in the onboarding overlay
- [x] 1.3 Surface capture-failure / denied state from `realtime.rs`/`ios_audio.rs` to the UI — `mic_capture`/VPIO hard-failures now emit a status; permission-denied is gated before connect

## 2. Onboarding surface

- [x] 2.1 First-run onboarding screen(s): what Nova is/does, permission priming — opaque `if VS.onboarding-open` overlay in `ui/app.slint`
- [x] 2.2 Persist "onboarding seen" — `setting_onboarding_seen.txt` via `save-setting`; read on boot to decide whether to show the overlay

## 3. Denied/blocked UI

- [x] 3.1 Persistent "microphone blocked — enable in Settings" state on the Talk surface — status label reads `VS.mic-perm == 2`; onboarding shows an "Open Settings" affordance
- [x] 3.2 Connect action reflects a blocked mic instead of silently failing — `on_realtime_toggle` gate: denied → Settings, undetermined → prompt, granted → connect

## 4. Follow-ups (separate changes, noted)

- [ ] 4.1 Manual text/search entry as a voice fallback
- [ ] 4.2 Saved places + history; account/settings depth
