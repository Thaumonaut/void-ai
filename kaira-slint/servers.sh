#!/usr/bin/env bash
# Start the Opus test servers on this computer. Server-based engines (Soniox is
# direct-to-cloud, but the server-Opus paths) reach these over your WiFi:
#   :8770  fixed-clip Opus stream  (Opus 3G mode)
#   :8771  streaming-TTS           (round-trip "server Opus" reply delivery)
# Keep this running while you test. The phone must be on the SAME WiFi, and its
# /sdcard/kaira/opus_server.txt must point at this computer's LAN IP (deploy.sh sets it).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OT="$HERE/../compare/opus_test"

pkill -f opus_stream_srv.py 2>/dev/null || true
pkill -f opus_tts_srv.py 2>/dev/null || true
cd "$OT"
python3 opus_stream_srv.py src6.wav 8770 &
python3 opus_tts_srv.py 8771 &
LAN=$(ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || echo '?')
echo "Opus servers up on $LAN — :8770 (clip) + :8771 (streaming TTS). Ctrl-C to stop."
trap 'pkill -f opus_stream_srv.py; pkill -f opus_tts_srv.py; echo; echo stopped.' INT TERM
wait
