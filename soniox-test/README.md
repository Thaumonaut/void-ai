# Soniox STT + TTS test (Indonesian + English)

Minimal, dependency-light tests of Soniox for **both** speech-to-text and
text-to-speech, focused on Bahasa Indonesia + English code-switching for Kaira.

- **STT** — `stt-rt-v5`, real-time WebSocket, `language_hints=["id","en"]`,
  per-token language ID to reveal ID↔EN code-switch. `wss://stt-rt.soniox.com`.
- **TTS** — `tts-rt-v1`, real-time WebSocket, cross-lingual voices via `language`.
  `wss://tts-rt.soniox.com`. ~$0.70/hr generated speech.

## Setup (one step)

```
cp .env.example .env       # then paste your Soniox API key into .env
```

Uses the existing Python 3.11 venv (`../pipecat-bench/.venv`, has `websockets`).

## Run

```
PY=../pipecat-bench/.venv/bin/python

# STT — transcribe the Indonesian sample clips (with language timeline)
$PY soniox_stt.py ../voicelab/id_sample.wav ../voicelab/id_sample2.wav

# STT — also stream an English translation
$PY soniox_stt.py ../voicelab/id_sample.wav --translate

# TTS — synthesize Indonesian to a wav you can play
$PY soniox_tts.py "Halo, saya Kaira. Ada yang bisa saya bantu hari ini?" id_out.wav --lang id --voice Maya

# Round-trip: TTS a code-switch line, then STT it back
$PY soniox_tts.py "Halo, tunggu sebentar, I am checking your balance sekarang." cs_out.wav --lang id
$PY soniox_stt.py cs_out.wav
```

The API key is read from `$SONIOX_API_KEY` or `.env`; it is never committed or
hardcoded. `.env` stays local.
