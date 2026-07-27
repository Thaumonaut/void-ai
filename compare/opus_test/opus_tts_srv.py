#!/usr/bin/env python3
"""Streaming-TTS server for the round-trip test: receives the LLM reply text,
synthesizes it on the host (Piper) → Opus 24k → streams it back over a simulated
3G downlink cap. Models the real server-driven TTS path.

Protocol (all little-endian):
  client -> [u32 cap_kbps][u32 opus_bitrate][u32 lang (0=en,1=id)][u32 text_len][text utf8]
  server -> repeated [u32 len][len bytes opus packet], then [u32 0] EOF
"""
import socket, struct, subprocess, tempfile, os, time, threading, sys

BASE = "/Users/jek/Documents/Projects/Personal/Rust-Mobile"
SYNTH = f"{BASE}/voicelab/target/release/examples/tts_synth"
DYLD = f"{BASE}/kaira-slint/target/debug:{BASE}/kaira-slint/target/debug/examples"
PIPER = {
    0: (f"{BASE}/voicelab/vits-piper-en_US-lessac-medium", "en"),
    1: (f"{BASE}/voicelab/vits-piper-id_ID-news_tts-medium-int8", "id"),
}
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8771


def ogg_opus_packets(path):
    d = open(path, "rb").read()
    pkts, i, cur = [], 0, b""
    while i < len(d):
        n = d[i + 26]
        laces = d[i + 27:i + 27 + n]
        off = i + 27 + n
        for lace in laces:
            cur += d[off:off + lace]; off += lace
            if lace < 255:
                pkts.append(cur); cur = b""
        i = off
    return pkts[2:]  # drop OpusHead + OpusTags


def synth_to_opus(text, lang, bitrate):
    model, langtag = PIPER.get(lang, PIPER[0])
    with tempfile.TemporaryDirectory() as td:
        wav, opus = f"{td}/r.wav", f"{td}/r.opus"
        env = dict(os.environ, DYLD_LIBRARY_PATH=DYLD)
        t0 = time.time()
        subprocess.run([SYNTH, "piper", model, text, wav, langtag],
                       env=env, capture_output=True, timeout=60)
        subprocess.run(["opusenc", "--quiet", "--bitrate", str(bitrate), "--framesize", "20",
                        "--downmix-mono", "--cvbr", wav, opus], capture_output=True, timeout=30)
        synth_ms = (time.time() - t0) * 1000
        return ogg_opus_packets(opus), synth_ms


def handle(conn):
    try:
        hdr = b""
        while len(hdr) < 16:
            chunk = conn.recv(16 - len(hdr))
            if not chunk: return
            hdr += chunk
        cap_kbps, bitrate, lang, tlen = struct.unpack("<IIII", hdr)
        bitrate = max(6, min(64, bitrate or 24))
        text = b""
        while len(text) < tlen:
            chunk = conn.recv(tlen - len(text))
            if not chunk: return
            text += chunk
        text = text.decode("utf-8", "replace")[:500]
        pkts, synth_ms = synth_to_opus(text, lang, bitrate)
        print(f"synth '{text[:40]}…' lang={lang} @{bitrate}k -> {len(pkts)} pkts in {synth_ms:.0f}ms, cap {cap_kbps}k")
        bps = cap_kbps * 1000 / 8 if cap_kbps > 0 else 0
        sent, t0 = 0, time.time()
        for p in pkts:
            frame = struct.pack("<I", len(p)) + p
            conn.sendall(frame); sent += len(frame)
            if bps > 0:
                slack = sent / bps - (time.time() - t0)
                if slack > 0: time.sleep(slack)
        conn.sendall(struct.pack("<I", 0))
    except (BrokenPipeError, ConnectionResetError, socket.timeout):
        pass
    except Exception as e:
        print("err:", e)
    finally:
        conn.close()


s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("0.0.0.0", PORT)); s.listen(5)
print(f"streaming-TTS server on :{PORT} (synth Piper -> Opus 24k -> paced stream)")
while True:
    conn, _ = s.accept()
    threading.Thread(target=handle, args=(conn,), daemon=True).start()
