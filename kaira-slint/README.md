# Kaira Voice Lab

A native-Android test harness (Slint UI, no WebView) for measuring **on-device and
server STT/TTS** for the Kaira voice agent — latency first, quality second, on
old/mid Android over spotty networks. Targets **arm64, Android 8+, 2 GB RAM**.

> Testing is **English-first**: you speak English; Indonesian is available to
> *listen* to on the TTS side (for later), not to speak.

---

## Deploy to your phone (the short version)

1. **On the phone:** enable *Developer options* → *USB debugging*, plug into the computer, tap *Allow* on the prompt.
2. **On the computer:**
   ```bash
   adb devices                      # confirm your phone shows up
   cd kaira-slint
   ./deploy.sh                      # deploys to the one connected device
   #   ./deploy.sh <serial>         # if more than one device is connected
   #   HEAVY=1 ./deploy.sh <serial> # also push Qwen3/Omni STT — needs 3 GB+ RAM
   ```
   `deploy.sh` installs the app, pushes the models, grants permissions, and writes
   the on-device config (API keys from `.env.local`, plus this computer's LAN IP so
   the phone can reach the Opus servers over WiFi).
3. **For server engines** (Soniox, "server Opus" reply delivery), also run:
   ```bash
   ./servers.sh                     # keep running; phone must be on the SAME WiFi
   ```

That's it — open **Voice Lab** on the phone.

### Keys
API keys are read from `Rust-Mobile/.env.local` at deploy time and pushed to the
phone; they are **never stored in this repo**. Expected keys:
```
OPENROUTER_KEY=sk-or-...        # round-trip LLM (Gemma 4 via OpenRouter)
SONIOX_API_KEY=...              # optional — Soniox STT/TTS engines
```
Rotate these if they leak; re-run `deploy.sh` to update the phone.

---

## The engines

**STT — speech → text** (you test these in English)

| Engine | Where | Size | Languages | Notes |
|---|---|---|---|---|
| Moonshine·EN | on-device | 42 MB | English only | fastest; returns blank on non-English |
| Whisper·multi | on-device | ~103 MB (int8) | multilingual — EN good, ID weak | |
| Omni·CTC | on-device | 366 MB | multilingual — good Indonesian | fits 2 GB |
| Qwen3·multi | on-device | ~980 MB | multilingual — best Indonesian | needs **3 GB+** (not pushed by default) |
| Soniox·cloud | **server** | — (cloud) | ID + EN + code-switch | needs key + network |
| Native·Android | on-device | 0 MB | phone's locale packs | Android's built-in `SpeechRecognizer` — no model to push; needs Google STT + the language pack installed. Defaults to `en-US`; drop `/sdcard/kaira/native_locale.txt` with e.g. `id-ID` to switch. Captures the mic itself, so it's single-mode/round-trip only (not in STT-compare). |

**TTS — text → speech** (English to test; Indonesian to listen to)

| Engine | Where | Size | Languages |
|---|---|---|---|
| Supertonic·EN | on-device | 139 MB | English (multilingual model) |
| Piper·ID | on-device | 36 MB | Indonesian |
| MMS·ID | on-device | 109 MB | Indonesian (16 kHz) |
| Soniox·EN / Soniox·ID | **server** | — (cloud) | English / Indonesian |
| server Opus reply | **server** | — | host Piper (EN/ID) → Opus stream |

**On-device** = runs entirely on the phone, no network, no Opus. **Server** = audio
crosses the network; the **Opus bitrate** setting applies (ignored for on-device).

---

## Modes

- **Single engine** — pick one STT + one TTS and test each on its own.
- **STT compare** — record once; every STT engine transcribes the same audio; mark which were right.
- **TTS compare** — one line spoken by every TTS engine, back-to-back; shows latency, replay each.
- **Round-trip** — speak → STT → LLM (Gemma 4) → TTS. Reply delivered **on-device** or as a **server Opus stream** over a simulated cell; shows `STT + LLM + TTS = time-to-first-reply`.
- **Opus 3G** — stream a fixed clip at a simulated downlink and measure rebuffer (proves what survives 3G).

---

## Build from source
Needs the Android NDK + the sherpa-onnx runtime libs (in `runtime_libs/arm64-v8a`)
and the prebuilt `third_party/opus-android/libopus.a`.
```bash
export ANDROID_HOME=~/Library/Android/sdk
export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/28.2.13676358
export SHERPA_ONNX_LIB_DIR="$PWD/runtime_libs/arm64-v8a"
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
cargo apk build --release --target aarch64-linux-android --lib
# APK: target/release/apk/VoiceLab.apk
```

---

## Troubleshooting

- **App shows old modes / no models** — you deployed to the emulator, not the phone, or the model folder is empty. Re-run `./deploy.sh <serial>` for the phone.
- **Server engine errors (connect / host / timeout)** — the phone can't reach this computer. Check: (1) `./servers.sh` is running, (2) phone + computer on the **same WiFi**, (3) `adb shell cat /sdcard/kaira/opus_server.txt` shows this computer's current LAN IP (re-run `deploy.sh` if your IP changed).
- **Soniox errors** — needs `SONIOX_API_KEY` in `.env.local` (then re-deploy) + network.
- **Qwen3/Omni crash on launch/run** — not enough RAM (need 3 GB+). Use the lighter engines.
- **Mic does nothing** — grant the mic permission: `adb shell pm grant com.kaira.voicelab android.permission.RECORD_AUDIO`.
