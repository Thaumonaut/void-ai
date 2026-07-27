#!/usr/bin/env python3
# Streams length-prefixed raw Opus packets (20ms each) of a fixed speech clip,
# encoded on-the-fly at the client-requested bitrate, paced to a client-requested
# downlink cap (simulated 3G). Protocol:
#   client -> [u32 cap_kbps (0=unlimited)][u32 opus_bitrate_kbps]
#   server -> repeated [u32 len][len bytes opus packet], then [u32 0] EOF
import socket, struct, subprocess, tempfile, sys, time, threading, os

SRC_WAV = sys.argv[1] if len(sys.argv) > 1 else "src6.wav"
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 8770
_cache = {}  # bitrate -> [packets], so repeat requests don't re-encode


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
    return pkts[2:]


def packets_for(bitrate):
    if bitrate not in _cache:
        with tempfile.TemporaryDirectory() as td:
            out = f"{td}/s.opus"
            subprocess.run(["opusenc", "--quiet", "--bitrate", str(bitrate), "--framesize", "20",
                            "--downmix-mono", "--cvbr", SRC_WAV, out], capture_output=True, timeout=30)
            _cache[bitrate] = ogg_opus_packets(out)
    return _cache[bitrate]


def handle(conn):
    try:
        hdr = b""
        while len(hdr) < 8:
            chunk = conn.recv(8 - len(hdr))
            if not chunk: return
            hdr += chunk
        cap_kbps, bitrate = struct.unpack("<II", hdr)
        bitrate = max(6, min(64, bitrate or 24))
        pkts = packets_for(bitrate)
        print(f"stream {len(pkts)} pkts @ {bitrate}k, cap {cap_kbps}k")
        bps = cap_kbps * 1000 / 8 if cap_kbps > 0 else 0
        sent, t0 = 0, time.time()
        for p in pkts:
            frame = struct.pack("<I", len(p)) + p
            conn.sendall(frame); sent += len(frame)
            if bps > 0:
                slack = sent / bps - (time.time() - t0)
                if slack > 0: time.sleep(slack)
        conn.sendall(struct.pack("<I", 0))
    except (BrokenPipeError, ConnectionResetError):
        pass
    finally:
        conn.close()


s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("0.0.0.0", PORT)); s.listen(5)
print(f"Opus clip server on :{PORT} (source {os.path.basename(SRC_WAV)}, on-the-fly bitrate)")
while True:
    conn, _ = s.accept()
    threading.Thread(target=handle, args=(conn,), daemon=True).start()
