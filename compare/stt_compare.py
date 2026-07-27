#!/usr/bin/env python3
"""Compare STT engines on the SAME audio, side by side:
  • on-device native engines — run on the connected Android device via adb + asr_bench
  • cloud engines — Soniox (extensible)

Then mark which ones got it right; a running scoreboard accumulates across runs.

  python3 stt_compare.py <wav> [--ref "ground truth"] [--engines qwen3,omni,...]

--ref  : auto-score (normalized match) instead of interactive marking (non-TTY).
No --ref: interactive — you type y/n per engine (run it in your own terminal).
"""
import subprocess, sys, os, json, re, time, difflib

ADB   = os.environ.get("ADB", "/Users/jek/Library/Android/sdk/platform-tools/adb")
BASE  = "/Users/jek/Documents/Projects/Personal/Rust-Mobile"
DEVD  = "/data/local/tmp/vl"
PY    = f"{BASE}/pipecat-bench/.venv/bin/python"
SONIO = f"{BASE}/soniox-test/soniox_stt.py"
HERE  = os.path.dirname(os.path.abspath(__file__))

# label, asr_bench engine arg, on-device model dir
ON_DEVICE = [
    ("Moonshine·EN", "moonshine", "sherpa-onnx-moonshine-tiny-en-quantized-2026-02-27"),
    ("Whisper·tiny", "whisper",   "sherpa-onnx-whisper-tiny"),
    ("Whisper·base", "whisper",   "sherpa-onnx-whisper-base"),
    ("Omni·CTC",     "omni",      "sherpa-onnx-omnilingual-asr-1600-languages-300M-ctc-v2-int8-2026-02-05"),
    ("Dolphin·CTC",  "dolphin",   "sherpa-onnx-dolphin-base-ctc-multi-lang-int8-2025-04-02"),
    ("Qwen3·multi",  "qwen3",     "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25"),
]

def sh(cmd, t=180):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=t)

def adb(*a, t=180):
    return sh([ADB, *a], t)

def norm(s):
    # non-alphanumeric → space (so "there—the" becomes "there the", not "therethe")
    return re.sub(r"\s+", " ", re.sub(r"[^a-z0-9]+", " ", (s or "").lower())).strip()

def similarity(ref, hit):
    a, b = norm(ref), norm(hit)
    if not b:
        return 0.0
    return difflib.SequenceMatcher(None, a, b).ratio()

def run_ondevice(engine, mdir, wavname):
    cmd = f"cd {DEVD} && LD_LIBRARY_PATH=. ./asr_bench {engine} {mdir} {wavname} 4 2>/dev/null"
    out = adb("shell", cmd).stdout
    m  = re.search(r"decode=([\d.]+)\s+rtf=([\d.]+)", out)
    tm = re.search(r"^TEXT (.*)$", out, re.M)
    return (tm.group(1).strip() if tm else ""), (float(m.group(1)) if m else None)

def run_soniox(wavpath):
    t0 = time.time()
    out = sh([PY, SONIO, wavpath]).stdout
    dt = time.time() - t0
    tm = re.search(r"transcript:\s*(.*)", out)
    return (tm.group(1).strip() if tm else ""), dt

def main():
    if len(sys.argv) < 2:
        print(__doc__); sys.exit(1)
    wav = sys.argv[1]
    ref = sys.argv[sys.argv.index("--ref") + 1] if "--ref" in sys.argv else None
    only = sys.argv[sys.argv.index("--engines") + 1].split(",") if "--engines" in sys.argv else None

    # stage the audio on device
    wavname = "compare_input.wav"
    if adb("push", wav, f"{DEVD}/{wavname}").returncode != 0:
        print("⚠ adb push failed — is a device connected + /data/local/tmp/vl set up?"); sys.exit(1)
    adb("shell", f"chmod 777 {DEVD}/{wavname}")
    present = set(adb("shell", f"ls {DEVD}").stdout.split())

    print(f"\n  audio : {wav}")
    if ref: print(f"  ref   : {ref}")
    print()
    results = []
    for label, engine, mdir in ON_DEVICE:
        if mdir not in present: continue
        if only and engine not in only and label not in only: continue
        text, dec = run_ondevice(engine, mdir, wavname)
        results.append({"engine": label, "src": "device", "text": text, "time": dec})
    text, dt = run_soniox(wav)
    results.append({"engine": "Soniox", "src": "cloud", "text": text, "time": dt})

    print(f"  {'ENGINE':<14}{'SRC':<8}{'TIME':<9}TRANSCRIPT")
    print(f"  {'-'*13:<14}{'-'*7:<8}{'-'*8:<9}{'-'*40}")
    for r in results:
        t = f"{r['time']:.2f}s" if r['time'] is not None else "—"
        print(f"  {r['engine']:<14}{r['src']:<8}{t:<9}{r['text'][:66] or '(empty)'}")

    # score
    sb_path = os.path.join(HERE, "stt_scoreboard.json")
    sb = json.load(open(sb_path)) if os.path.exists(sb_path) else {}
    print()
    for r in results:
        if ref is not None:
            sim = similarity(ref, r["text"])
            ok = sim >= 0.90          # "essentially correct"
            print(f"  {'✓' if ok else '✗'} {r['engine']:<14} {sim*100:.0f}% match")
        else:
            ans = input(f"  {r['engine']:<14} correct? [y/N/skip]  → ").strip().lower()
            if ans in ("s", "skip"): continue
            ok = ans in ("y", "yes")
        e = r["engine"]; sb.setdefault(e, {"correct": 0, "total": 0})
        sb[e]["total"] += 1; sb[e]["correct"] += 1 if ok else 0
    json.dump(sb, open(sb_path, "w"), indent=2)

    print("\n  SCOREBOARD (cumulative across runs)")
    for e, v in sorted(sb.items(), key=lambda x: -x[1]["correct"] / max(1, x[1]["total"])):
        pct = 100 * v["correct"] / max(1, v["total"])
        print(f"    {e:<14} {v['correct']}/{v['total']}  ({pct:.0f}%)")

if __name__ == "__main__":
    main()
