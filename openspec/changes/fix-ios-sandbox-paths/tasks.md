## 1. Implement the iOS path

- [ ] 1.1 Add an iOS branch to `tts::files_dir()` returning the app container (Documents / Application Support) via a small objc2 helper (`NSSearchPathForDirectoriesInDomains`)
- [ ] 1.2 Ensure the directory exists (create if missing) and is writable

## 2. Audit + verify consumers

- [ ] 2.1 Settings read/write (`lib.rs` `on_save_setting`, restore-settings)
- [ ] 2.2 `realtime_url.txt` backend override
- [ ] 2.3 Other consumers: `lean_server`, `soniox_key`, `opus_server.txt`, `native_locale.txt`, STT model dir
- [ ] 2.4 Verify persistence + override work on a real iOS device (they currently only work in the sim)

## 3. Cleanup

- [ ] 3.1 Remove developer-absolute path defaults; keep any sim-only host access explicitly gated
