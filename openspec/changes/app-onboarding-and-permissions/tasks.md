## 1. Permission plumbing

- [ ] 1.1 Request microphone permission in-app (iOS `AVAudioSession.requestRecordPermission` / Android runtime permission) — remove the reliance on `adb grant`
- [ ] 1.2 Request location permission in-app with rationale
- [ ] 1.3 Surface capture-failure / denied state from `realtime.rs`/`ios_audio.rs` to the UI

## 2. Onboarding surface

- [ ] 2.1 First-run onboarding screen(s): what Nova is/does, permission priming
- [ ] 2.2 Persist "onboarding seen"

## 3. Denied/blocked UI

- [ ] 3.1 Persistent "microphone blocked — enable in Settings" state on the Talk surface
- [ ] 3.2 Connect action reflects a blocked mic instead of silently failing

## 4. Follow-ups (separate changes, noted)

- [ ] 4.1 Manual text/search entry as a voice fallback
- [ ] 4.2 Saved places + history; account/settings depth
