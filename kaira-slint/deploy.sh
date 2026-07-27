#!/usr/bin/env bash
# Deploy the Kaira voice-lab to a connected Android phone (arm64-v8a, Android 8+).
# Installs the APK, pushes the on-device models, grants permissions, and writes
# the on-device config (keys + lean-server URL + Opus-server LAN IP). Secrets are
# read from ../.env.local at deploy time and pushed via temp files — never stored
# in the repo.
#
#   ./deploy.sh                    deploy to the only connected device
#   ./deploy.sh <serial>           pick a device  (list them: adb devices)
#   HEAVY=1 ./deploy.sh <serial>   also push heavy STT (Qwen3/Omni) — needs 3 GB+ RAM
#   LAN_IP=1.2.3.4 ./deploy.sh     override the auto-detected computer IP
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
ADB="${ADB:-adb}"
APK="$HERE/target/release/apk/VoiceLab.apk"
MODELS="$ROOT/voicelab"
ENVFILE="$ROOT/.env.local"
LEAN="https://micro-agent-lean.fly.dev"

# Target a specific device via adb's own env var (works with an empty value too,
# so no fragile "${arr[@]}" expansion under bash 3.2 + set -u).
if [ -n "${1:-}" ]; then export ANDROID_SERIAL="$1"; fi

$ADB get-state >/dev/null 2>&1 || {
  echo "No device. Plug in a phone, enable USB debugging, and run 'adb devices'."; exit 1; }
MODEL=$($ADB shell getprop ro.product.model | tr -d '\r')
echo "▶ deploying to: $MODEL"

[ -f "$APK" ] || { echo "APK not found: $APK  (build it — see README.md)"; exit 1; }
echo "▶ installing APK…"; $ADB install -r "$APK" | tail -1

# permissions (Android 8–10 need READ_EXTERNAL_STORAGE; 11+ also MANAGE)
$ADB shell pm grant com.kaira.voicelab android.permission.RECORD_AUDIO 2>/dev/null || true
$ADB shell pm grant com.kaira.voicelab android.permission.READ_EXTERNAL_STORAGE 2>/dev/null || true
$ADB shell appops set com.kaira.voicelab MANAGE_EXTERNAL_STORAGE allow 2>/dev/null || true
# Some OEMs (OnePlus/OxygenOS…) block adb from granting runtime permissions.
MIC=$($ADB shell dumpsys package com.kaira.voicelab 2>/dev/null | grep "RECORD_AUDIO: granted" | head -1)
if ! echo "$MIC" | grep -q "granted=true"; then
  echo "  ⚠ could not grant the MIC permission via adb (OEM-locked)."
  echo "    Grant it by hand: Settings → Apps → Voice Lab → Permissions → Microphone → Allow"
  echo "    (opening that screen now), then also allow Files/Storage there."
  $ADB shell am start -a android.settings.APPLICATION_DETAILS_SETTINGS -d package:com.kaira.voicelab >/dev/null 2>&1 || true
  echo "    Tip: to let adb grant it automatically, enable 'USB debugging (Security settings)' in Developer options + re-run."
fi

# models
$ADB shell mkdir -p /sdcard/kaira
CORE=(sherpa-onnx-moonshine-tiny-en-quantized-2026-02-27 sherpa-onnx-whisper-tiny \
      vits-piper-id_ID-news_tts-medium-int8 sherpa-onnx-supertonic-3-tts-int8-2026-05-11 vits-mms-ind)
HEAVY_M=(sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25 \
         sherpa-onnx-omnilingual-asr-1600-languages-300M-ctc-v2-int8-2026-02-05)
LIST=("${CORE[@]}"); [ "${HEAVY:-0}" = 1 ] && LIST+=("${HEAVY_M[@]}")
for m in "${LIST[@]}"; do
  if [ -d "$MODELS/$m" ]; then echo "> push ${m}"; $ADB push "$MODELS/$m" /sdcard/kaira/ | tail -1
  else echo "  skip (not staged in voicelab/): ${m}"; fi
done

# secrets + config (from .env.local, pushed via temp files, never persisted here)
push_secret() { local t; t=$(mktemp); printf '%s' "$2" > "$t"; $ADB push "$t" "/sdcard/kaira/$1" >/dev/null; rm -f "$t"; }
if [ -f "$ENVFILE" ]; then
  SX=$(grep -E '^SONIOX_API_KEY=' "$ENVFILE" | head -1 | cut -d= -f2-)
  if [ -n "$SX" ]; then push_secret soniox_key.txt "$SX"; echo "▶ Soniox key set (for Soniox TTS)"; else echo "  ⚠ no SONIOX_API_KEY in .env.local — Soniox TTS disabled"; fi
else
  echo "  ⚠ no .env.local at $ENVFILE — Soniox TTS disabled"
fi

# Backend URL — round-trip STT (Soniox token) + LLM (/chat) + TTS (/tts Opus) all
# go through the lean server, so no LLM/Soniox key is needed on the device for them.
push_secret lean_server.txt "$LEAN"; echo "▶ lean server = $LEAN"

# Opus 3G mode (local codec/network sim) still needs the Mac's server on the same WiFi.
LAN_IP="${LAN_IP:-$(ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || echo '')}"
if [ -n "$LAN_IP" ]; then push_secret opus_server.txt "$LAN_IP:8770"; echo "▶ Opus 3G server = $LAN_IP:8770 (local, same-WiFi)"
else echo "  ⚠ couldn't detect this computer's LAN IP — Opus 3G mode: set LAN_IP=<ip> ./deploy.sh"; fi

echo
echo "✓ $MODEL is ready."
echo "  Round-trip + Soniox go straight to the lean server (fly) — no local setup needed."
echo "  Only the Opus 3G mode needs the local servers on this computer:  ./servers.sh"
