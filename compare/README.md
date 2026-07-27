# Kaira engine compare tools

Host-side A/B harnesses for picking Kaira's STT + TTS engines. They fan the SAME
input through every wired engine (on-device native + Soniox cloud) so you can
compare accuracy, latency, and quality directly. Extensible — add engines in the
`CFG`/`ON_DEVICE` lists at the top of each script.

Prereqs: a connected Android device/emulator with `/data/local/tmp/vl` staged
(asr_bench binary + runtime `.so` + model dirs — see the harness memory), and a
Soniox key in `../soniox-test/.env`.

## STT compare — "which engines got it right"

Fans audio through the on-device engines (Moonshine / Whisper / Omni / Dolphin /
Qwen3, run on the phone via `adb` + `asr_bench`) + Soniox cloud, shows all
transcripts + decode times, then you mark which are correct. A cumulative
scoreboard (`stt_scoreboard.json`) builds up across runs.

```
# interactive — run in YOUR terminal so you can type y/n per engine:
python3 stt_compare.py ../voicelab/en_sample.wav

# auto-score against a reference (non-interactive, for scripting):
python3 stt_compare.py <wav> --ref "the exact words spoken"

# only some engines:
python3 stt_compare.py <wav> --engines qwen3,omni,Soniox
```

`--ref` marks ✓ when similarity ≥ 90% (tolerant of punctuation / one-word slips);
without `--ref` you judge each by ear/eye.

## TTS compare — "latency vs quality for the same line"

Synthesizes one line with each engine (Piper + Supertonic on host, Soniox cloud),
prints time-to-first-audio + total + RTF, and plays each clip so you can rate the
quality tradeoff.

```
python3 tts_compare.py "Hello, I'm Kaira. How can I help you today?" --lang en
python3 tts_compare.py "Halo, ada yang bisa saya bantu?" --lang id
python3 tts_compare.py "..." --lang en --no-play      # skip playback
```

Clips are saved in `tts_out/` (replay with `afplay tts_out/<clip>.wav`).

Caveats: on-device TTS timings are HOST-side (near-native → optimistic vs a real
A53); the audio quality is identical to on-device (same model). STT runs on the
actual device. Soniox timings include the network round-trip.

## Adding a cloud engine
Wire a new provider (Deepgram, Google, ElevenLabs…) by adding a `run_<name>()`
that returns `{text,time}` (STT) or `{first,total,dur,rtf}` (TTS) and appending it
to the results loop. Keys go in env/.env; never hardcode.
