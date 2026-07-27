## Why

`tts::files_dir()` returns a hardcoded macOS host path (`/Users/jek/.../voicelab`) on non-Android. It *works in the iOS simulator* (which can read host paths) but the sandbox on a real device cannot — so it **silently fails on every physical device**: settings persistence and the backend-URL override are dead, yet everything passes in the sim. A nasty dev/device divergence trap. (Audit P0.)

## What Changes

- Implement an **iOS branch** of `files_dir()` returning the app's writable container (Documents / Application Support) instead of the host path.
- Audit every consumer of `files_dir()` (settings read/write, `realtime_url.txt`, `lean_server`, `soniox_key`, `opus_server`, `native_locale`, STT model dir) and confirm each works on-device.
- Remove developer-absolute paths from runtime defaults; keep the sim-readable behavior only where explicitly intended.

## Capabilities

### New Capabilities
- `device-storage-paths`: on-device reads/writes use the app's sandbox container so persistence and config overrides work on real hardware, not just the simulator.

## Impact

- `kaira-slint/src/tts.rs` (`files_dir`), plus every consumer in `lib.rs` (`on_save_setting`, restore-settings, `realtime_url.txt` override, `opus_server.txt`), `stt.rs`, `soniox`/`lean_server`. Likely a small objc2 helper (`NSSearchPathForDirectoriesInDomains` / `FileManager`).
- Non-goals: changing what settings exist (see `add-ui-states`/onboarding).
