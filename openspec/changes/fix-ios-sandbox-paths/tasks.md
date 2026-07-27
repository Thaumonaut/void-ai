## 1. Implement the iOS path

- [x] 1.1 Add an iOS branch to `tts::files_dir()` returning the app container Documents (`$HOME/Documents` — iOS sets HOME to the sandbox; no objc2 needed)
- [x] 1.2 Directory is guaranteed by iOS (Documents always exists); `temp_dir()` fallback if HOME is unset

## 2. Audit + verify consumers

- [x] 2.1 Settings read/write (`lib.rs` `on_save_setting`, restore-settings) — verified: theme setting written to the container is read on launch (sim)
- [x] 2.2 `realtime_url.txt` backend override (same `files_dir()` — fixed by extension)
- [x] 2.3 Other consumers (`lean_server`, `soniox_key`, `opus_server.txt`, `native_locale.txt`, STT model dir) all route through `files_dir()`
- [ ] 2.4 Verify persistence + `realtime_url.txt` override on a REAL iOS device (mechanism proven on the sim)

## 3. Cleanup

- [ ] 3.1 Desktop-dev branch still uses an absolute host path (`voicelab`) — dev-only; make it relative/configurable when convenient
