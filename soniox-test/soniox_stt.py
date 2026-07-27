#!/usr/bin/env python3
"""Test Soniox real-time STT on a wav file, with Indonesian + English language
hints and per-token language identification (to reveal ID<->EN code-switch).

  python soniox_stt.py <wav> [--translate]

--translate also streams an English translation (Soniox one-way translation).
Prints the final transcript, a language timeline, and latency stats.
"""
import asyncio
import json
import sys
import time

import websockets
from _common import load_api_key, wav_to_pcm16_mono

STT_URL = "wss://stt-rt.soniox.com/transcribe-websocket"
RATE = 16000
CHUNK = int(RATE * 0.1) * 2  # 100 ms of 16-bit mono


async def transcribe(path, translate=False):
    api_key = load_api_key()
    pcm = wav_to_pcm16_mono(path, RATE)
    dur = len(pcm) / 2 / RATE

    config = {
        "api_key": api_key,
        "model": "stt-rt-v5",
        "audio_format": "pcm_s16le",
        "num_channels": 1,
        "sample_rate": RATE,
        "language_hints": ["id", "en"],
        "enable_language_identification": True,
    }
    if translate:
        config["translation"] = {"type": "one_way", "target_language": "en"}

    print(f"→ {path}  ({dur:.1f}s audio)  model=stt-rt-v5  hints=id,en")
    t0 = time.monotonic()
    first_token_at = None
    finals = []  # (text, language)

    async with websockets.connect(STT_URL, max_size=None) as ws:
        await ws.send(json.dumps(config))

        async def send_audio():
            try:
                for i in range(0, len(pcm), CHUNK):
                    await ws.send(pcm[i:i + CHUNK])
                    await asyncio.sleep(0.05)  # pace ~2x realtime
                await ws.send("")  # empty STRING signals end-of-audio (not b"")
            except websockets.exceptions.ConnectionClosed:
                pass  # server closed first (e.g. error) — receiver reports why

        sender = asyncio.create_task(send_audio())
        async for msg in ws:
            data = json.loads(msg)
            if data.get("error_code") or data.get("error_message"):
                print(f"  ⚠ server error {data.get('error_code')}: {data.get('error_message')}")
                break
            for tok in data.get("tokens", []):
                if tok.get("is_final"):
                    if first_token_at is None:
                        first_token_at = time.monotonic() - t0
                    finals.append((tok.get("text", ""), tok.get("language", "")))
            if data.get("finished"):
                break
        await sender

    text = "".join(t for t, _ in finals)
    # collapse the per-token language stream into contiguous spans
    spans, cur = [], None
    for t, lang in finals:
        if lang and lang != cur:
            spans.append(lang)
            cur = lang
    total = time.monotonic() - t0

    print("\n  transcript:", text.strip())
    print("  languages :", " → ".join(spans) if spans else "(none labeled)")
    ftt = f"{first_token_at:.2f}s" if first_token_at else "n/a"
    print(f"  first token {ftt} · total {total:.2f}s")
    return text


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    translate = "--translate" in sys.argv
    if not args:
        print(__doc__)
        sys.exit(1)
    for wav in args:
        asyncio.run(transcribe(wav, translate=translate))
        print()
