#!/usr/bin/env python3
"""Test Soniox real-time TTS: synthesize text to a wav so you can listen.

  python soniox_tts.py "Halo, saya Kaira." out.wav [--lang id] [--voice Maya]

Soniox voices are cross-lingual — the same voice speaks any of 60+ languages via
--lang. Use --lang id for Bahasa Indonesia. Reports time-to-first-audio.
"""
import asyncio
import base64
import json
import sys
import time

import websockets
from _common import load_api_key, write_wav

TTS_URL = "wss://tts-rt.soniox.com/tts-websocket"
RATE = 24000


def arg(flag, default):
    if flag in sys.argv:
        return sys.argv[sys.argv.index(flag) + 1]
    return default


async def synth(text, out, language="id", voice="Maya"):
    api_key = load_api_key()
    config = {
        "api_key": api_key,
        "model": "tts-rt-v1",
        "language": language,
        "voice": voice,
        "audio_format": "pcm_s16le",
        "sample_rate": RATE,
        "stream_id": "kaira-test",
    }
    print(f"→ synth [{language}/{voice}]: {text!r}")
    t0 = time.monotonic()
    first_audio_at = None
    chunks = []

    async with websockets.connect(TTS_URL, max_size=None) as ws:
        await ws.send(json.dumps(config))
        await ws.send(json.dumps({"text": text, "text_end": True, "stream_id": "kaira-test"}))
        async for msg in ws:
            data = json.loads(msg)
            if data.get("audio"):
                if first_audio_at is None:
                    first_audio_at = time.monotonic() - t0
                chunks.append(base64.b64decode(data["audio"]))
            if data.get("terminated"):
                break

    pcm = b"".join(chunks)
    write_wav(out, pcm, RATE)
    dur = len(pcm) / 2 / RATE
    total = time.monotonic() - t0
    fa = f"{first_audio_at:.2f}s" if first_audio_at else "n/a"
    print(f"  saved {out}  ({dur:.1f}s audio) · 1st audio {fa} · total {total:.2f}s")


if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)
    text, out = sys.argv[1], sys.argv[2]
    asyncio.run(synth(text, out, language=arg("--lang", "id"), voice=arg("--voice", "Maya")))
