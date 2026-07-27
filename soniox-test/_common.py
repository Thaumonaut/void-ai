"""Shared helpers for the Soniox STT/TTS tests: API-key loading + wav I/O."""
import os
import wave
import audioop  # stdlib in 3.11 (deprecated but present); removed in 3.13


def load_api_key():
    """Key from $SONIOX_API_KEY, else from a local .env file (KEY=VALUE)."""
    key = os.environ.get("SONIOX_API_KEY")
    if key:
        return key.strip()
    env_path = os.path.join(os.path.dirname(__file__), ".env")
    if os.path.exists(env_path):
        for line in open(env_path):
            line = line.strip()
            if line.startswith("SONIOX_API_KEY") and "=" in line:
                return line.split("=", 1)[1].strip().strip('"').strip("'")
    raise SystemExit(
        "No API key. Set SONIOX_API_KEY, or copy .env.example to .env and paste your key."
    )


def wav_to_pcm16_mono(path, target_rate=16000):
    """Read any PCM wav → raw 16-bit mono bytes at target_rate."""
    with wave.open(path, "rb") as w:
        ch, sw, fr = w.getnchannels(), w.getsampwidth(), w.getframerate()
        data = w.readframes(w.getnframes())
    if sw != 2:
        data = audioop.lin2lin(data, sw, 2)
        sw = 2
    if ch == 2:
        data = audioop.tomono(data, 2, 0.5, 0.5)
    if fr != target_rate:
        data, _ = audioop.ratecv(data, 2, 1, fr, target_rate, None)
    return data


def write_wav(path, pcm_bytes, rate):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(pcm_bytes)
