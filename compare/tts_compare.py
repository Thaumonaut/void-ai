#!/usr/bin/env python3
"""Compare TTS engines on the SAME line of text: on-device (Piper, Supertonic)
run on host + Soniox cloud. Reports time-to-first-audio + total per engine, and
plays each clip so you can judge latency-vs-quality.

  python3 tts_compare.py "some text" [--lang en|id] [--no-play]

Note: on-device timings are HOST-side (near-native) — optimistic vs a real A53;
cloud timings include the network round-trip. Use for relative latency + quality.
"""
import subprocess, sys, os, re

BASE  = "/Users/jek/Documents/Projects/Personal/Rust-Mobile"
VL    = f"{BASE}/voicelab"
SYNTH = f"{VL}/target/release/examples/tts_synth"
PY    = f"{BASE}/pipecat-bench/.venv/bin/python"
STTS  = f"{BASE}/soniox-test/soniox_tts.py"
HERE  = os.path.dirname(os.path.abspath(__file__))

CFG = {
    "en": [
        ("Piper·EN",      "device", ["piper",      f"{VL}/vits-piper-en_US-lessac-medium"]),
        ("Supertonic·EN", "device", ["supertonic", f"{VL}/sherpa-onnx-supertonic-3-tts-int8-2026-05-11"]),
        ("Soniox·EN",     "cloud",  None),
    ],
    "id": [
        ("Piper·ID",      "device", ["piper",      f"{VL}/vits-piper-id_ID-news_tts-medium-int8"]),
        ("Supertonic·ID", "device", ["supertonic", f"{VL}/sherpa-onnx-supertonic-3-tts-int8-2026-05-11"]),
        ("Soniox·ID",     "cloud",  None),
    ],
}

def run_device(args, text, out, lang):
    r = subprocess.run([SYNTH] + args + [text, out, lang], capture_output=True, text=True, timeout=180)
    m = re.search(r"first=([\d.]+) total=([\d.]+) dur=([\d.]+) rtf=([\d.]+)", r.stdout)
    return dict(first=float(m[1]), total=float(m[2]), dur=float(m[3]), rtf=float(m[4])) if m else None

def run_soniox(text, out, lang):
    r = subprocess.run([PY, STTS, text, out, "--lang", lang, "--voice", "Maya"],
                       capture_output=True, text=True, timeout=180)
    m  = re.search(r"1st audio ([\d.]+)s · total ([\d.]+)s", r.stdout)
    dm = re.search(r"\(([\d.]+)s audio\)", r.stdout)
    if not m:
        return None
    dur = float(dm[1]) if dm else None
    return dict(first=float(m[1]), total=float(m[2]), dur=dur,
                rtf=(float(m[2]) / dur if dur else None))

def main():
    if len(sys.argv) < 2:
        print(__doc__); sys.exit(1)
    text = sys.argv[1]
    lang = sys.argv[sys.argv.index("--lang") + 1] if "--lang" in sys.argv else "en"
    play = "--no-play" not in sys.argv
    outdir = os.path.join(HERE, "tts_out"); os.makedirs(outdir, exist_ok=True)

    print(f'\n  text : "{text}"   (lang={lang})\n')
    print(f"  {'ENGINE':<15}{'SRC':<8}{'1st-AUDIO':<11}{'TOTAL':<9}{'RTF':<7}{'DUR'}")
    print(f"  {'-'*14:<15}{'-'*7:<8}{'-'*10:<11}{'-'*8:<9}{'-'*6:<7}{'-'*5}")
    clips = []
    for label, src, args in CFG.get(lang, CFG["en"]):
        out = os.path.join(outdir, f"{label.replace('·','_')}.wav")
        res = run_soniox(text, out, lang) if src == "cloud" else run_device(args, text, out, lang)
        if not res:
            print(f"  {label:<15}{src:<8}(failed)"); continue
        clips.append((label, out))
        f = f"{res['first']:.2f}s" if res['first'] is not None else "—"
        t = f"{res['total']:.2f}s" if res['total'] is not None else "—"
        rt = f"{res['rtf']:.2f}" if res['rtf'] is not None else "—"
        d = f"{res['dur']:.1f}s" if res['dur'] is not None else "—"
        print(f"  {label:<15}{src:<8}{f:<11}{t:<9}{rt:<7}{d}")

    print(f"\n  clips saved in {outdir}")
    if play:
        print("  ▶ playing each (judge quality)…")
        for label, out in clips:
            print(f"    ▶ {label}")
            subprocess.run(["afplay", out])
    else:
        print("  (replay: afplay <clip>)")

if __name__ == "__main__":
    main()
