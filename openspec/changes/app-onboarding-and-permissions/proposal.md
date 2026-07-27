## Why

There is no first-run onboarding and no in-app microphone-permission flow. Today the mic is granted out-of-band via `adb ... grant RECORD_AUDIO` — a real installed app cannot do that, and if permission is denied the audio stream just fails to stderr with **no UI feedback**. A voice app whose mic is silently blocked is dead on arrival. (Audit P0 for shipping.)

## What Changes

- **First-run onboarding**: explain Nova + what the app does, and prime the **microphone** (and **location**) permissions with rationale before requesting them.
- **In-app permission handling**: request mic/location through the platform properly; when denied, show a persistent, explanatory "mic blocked — enable in Settings" state instead of failing silently.
- **Denied/blocked states** wired into the Talk surface so the connect action explains why it can't proceed.

## Capabilities

### New Capabilities
- `onboarding-and-permissions`: a first-time user is oriented and guided through granting mic/location, and a denied permission produces a clear, recoverable UI state rather than a silent failure.

## Impact

- `kaira-slint`: a new onboarding surface (Slint), platform permission requests (iOS `AVAudioSession`/CoreLocation, Android runtime permissions), denied-state UI on the Talk view; `src/realtime.rs`/`ios_audio.rs` surface capture-failure to the UI.
- Related follow-ups (separate changes): a manual text/search entry as a voice fallback; saved places + history; account/settings depth.
- Non-goals: those follow-ups; the permission-agnostic connection work (`harden-realtime-connection`).
