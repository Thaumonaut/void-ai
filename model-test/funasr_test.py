import sys, time, wave
from funasr import AutoModel

WAV = sys.argv[1] if len(sys.argv) > 1 else "../voicelab/id_sample.wav"
with wave.open(WAV) as w:
    audio_secs = w.getnframes() / w.getframerate()

print(f"loading Fun-ASR-MLT-Nano-2512 (downloads ~800M params first run)…")
t0 = time.time()
model = AutoModel(model="FunAudioLLM/Fun-ASR-MLT-Nano-2512", hub="hf", disable_update=True, device="cpu")
load = time.time() - t0
print(f"loaded in {load:.1f}s")

for lang in ("id", None):
    t1 = time.time()
    try:
        kw = {"language": lang} if lang else {}
        res = model.generate(input=WAV, **kw)
        dec = time.time() - t1
        text = res[0].get("text", "") if res else ""
        print(f"[lang={lang}] decode {dec:.2f}s  RTF {dec/audio_secs:.3f}")
        print(f"  text: {text}")
    except Exception as e:
        print(f"[lang={lang}] error: {e}")
