#!/usr/bin/env python3
"""
Kaira voice-to-voice latency benchmark client.

Connects to the SmallWebRTC server (server.py) as a real aiortc WebRTC peer, streams a
pre-recorded ID+EN utterance, and measures VOICE-TO-VOICE LATENCY:

    latency = (time first bot audio arrives) - (time the user utterance ended)

This is the number that matters for Kaira: it includes VAD end-of-speech detection +
STT + LLM + TTS + network round-trips + transport buffering -- i.e. everything the
user actually waits through. It reports P50/P95 over N turns.

Per-stage TTFB (STT vs LLM vs TTS) is emitted by the SERVER (PipelineParams
enable_metrics=True) into the server logs -- grep the server output for "TTFB". The
client cannot see per-stage numbers over plain WebRTC media; see README.

Usage:
    # 1) start the server with the greeting disabled so the only bot audio is the reply:
    #      GREETING_ENABLED=false python server.py
    # 2) run the benchmark:
    python bench.py --server http://localhost:7860 --wav audio/sample_id_en.wav -n 10

    python bench.py --make-sample          # write a placeholder wav (NOT real speech)
    python bench.py --self-test            # offline harness check, no server needed
    python bench.py --tc-help              # print 3G network-degradation (tc netem) recipe

Verified import surface against aiortc (installed via pipecat-ai[webrtc]).
"""

import argparse
import asyncio
import json
import math
import statistics
import sys
import time
import urllib.request
import wave

DEFAULT_WAV = "audio/sample_id_en.wav"


# ---------------------------------------------------------------------------
# Small helpers (no third-party deps)
# ---------------------------------------------------------------------------
def wav_duration_secs(path: str) -> float:
    with wave.open(path, "rb") as w:
        return w.getnframes() / float(w.getframerate())


def percentile(values, pct: float) -> float:
    """Linear-interpolation percentile (pct in 0..100). Pure python."""
    if not values:
        return float("nan")
    xs = sorted(values)
    if len(xs) == 1:
        return xs[0]
    k = (len(xs) - 1) * (pct / 100.0)
    lo = math.floor(k)
    hi = math.ceil(k)
    if lo == hi:
        return xs[int(k)]
    return xs[lo] * (hi - k) + xs[hi] * (k - lo)


def frame_rms(frame) -> float:
    """RMS energy of an av.AudioFrame of int16 PCM."""
    import numpy as np

    samples = frame.to_ndarray().astype("float64")
    if samples.size == 0:
        return 0.0
    return float(np.sqrt(np.mean(samples * samples)))


def make_placeholder_wav(path: str, seconds: float = 4.0, rate: int = 16000):
    """Write a valid 16kHz mono wav: silence, a burst of noise (stand-in for speech),
    then silence. This exercises the harness but WILL NOT transcribe to real words --
    replace it with a genuine Bahasa Indonesia + English recording for real numbers."""
    import os
    import random

    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    n = int(seconds * rate)
    frames = bytearray()
    for i in range(n):
        t = i / rate
        if 0.5 < t < seconds - 1.0:  # "speech" region
            v = int(random.uniform(-9000, 9000))
        else:
            v = 0
        frames += int(v).to_bytes(2, "little", signed=True)
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(bytes(frames))
    print(f"wrote placeholder wav: {path} ({seconds:.1f}s @ {rate}Hz)")
    print("NOTE: placeholder noise -- STT will not produce words. Replace with real speech.")


TC_HELP = """\
Network degradation (simulate poor Indonesian 3G)
=================================================
`tc netem` shapes an outbound interface on LINUX (not macOS). To be meaningful it must
run on a host that sits ON THE PATH to the server -- ideally the server box itself in
Singapore (fly.io `sin`), OR a Linux router/VM the client traffic passes through. Adding
loss/latency on your Mac's loopback does nothing useful.

Typical 3G profile (added, one direction):  ~300ms delay +/- 50ms jitter, 2% loss.

  # apply on the server's egress NIC (e.g. eth0):
  sudo tc qdisc add dev eth0 root netem delay 300ms 50ms distribution normal loss 2%

  # tighten to a bad-3G / congested profile:
  sudo tc qdisc change dev eth0 root netem delay 500ms 100ms loss 5% reorder 5% 50%

  # constrain bandwidth to ~384kbit with a queue (HSPA-ish), combine with the above:
  sudo tc qdisc add dev eth0 root handle 1: tbf rate 384kbit burst 32kbit latency 400ms

  # inspect / remove:
  tc qdisc show dev eth0
  sudo tc qdisc del dev eth0 root

Run the benchmark from the client while the shaper is active and compare P50/P95 to the
un-shaped baseline. The ONLY faithful test puts the shaper on the real path to Indonesia
(deploy to `sin`, run bench.py from a device in Indonesia), because WebRTC's jitter
buffer + packet loss recovery behave very differently from a clean localhost link.
"""


# ---------------------------------------------------------------------------
# One benchmark turn over WebRTC
# ---------------------------------------------------------------------------
async def run_one_turn(server: str, wav: str, ice_servers, rms_threshold: float,
                       response_timeout: float, settle_secs: float):
    """Returns voice-to-voice latency in seconds, or None on timeout/failure."""
    from aiortc import (
        RTCConfiguration,
        RTCIceServer,
        RTCPeerConnection,
        RTCSessionDescription,
    )
    from aiortc.contrib.media import MediaPlayer
    from aiortc.mediastreams import MediaStreamError

    loop = asyncio.get_event_loop()
    pc = RTCPeerConnection(RTCConfiguration(iceServers=ice_servers))

    player = MediaPlayer(wav)
    pc.addTrack(player.audio)

    duration = wav_duration_secs(wav)
    state = {"connected_at": None, "first_response_at": None}
    got_response = asyncio.Event()

    @pc.on("connectionstatechange")
    async def _on_conn_state():
        if pc.connectionState == "connected" and state["connected_at"] is None:
            state["connected_at"] = loop.time()
        elif pc.connectionState in ("failed", "closed"):
            got_response.set()

    async def consume(track):
        while True:
            try:
                frame = await track.recv()
            except MediaStreamError:
                break
            now = loop.time()
            # Only count energetic audio that arrives AFTER the user utterance ended.
            if state["connected_at"] is None:
                continue
            speech_end = state["connected_at"] + duration
            if now <= speech_end + settle_secs:
                continue
            if frame_rms(frame) >= rms_threshold and state["first_response_at"] is None:
                state["first_response_at"] = now
                got_response.set()
                break

    @pc.on("track")
    def _on_track(track):
        if track.kind == "audio":
            asyncio.ensure_future(consume(track))

    try:
        # --- offer / answer over the SmallWebRTC /api/offer endpoint ---
        await pc.setLocalDescription(await pc.createOffer())
        await _wait_ice_complete(pc)
        answer = await loop.run_in_executor(
            None, _post_offer, server, pc.localDescription.sdp, pc.localDescription.type
        )
        await pc.setRemoteDescription(
            RTCSessionDescription(sdp=answer["sdp"], type=answer["type"])
        )

        # Wait until connected, then for the response (bounded by timeout).
        deadline = duration + settle_secs + response_timeout
        try:
            await asyncio.wait_for(got_response.wait(), timeout=deadline + 5)
        except asyncio.TimeoutError:
            return None

        if state["connected_at"] is None or state["first_response_at"] is None:
            return None
        speech_end = state["connected_at"] + duration
        return state["first_response_at"] - speech_end
    finally:
        try:
            player.audio.stop()
        except Exception:
            pass
        await pc.close()


async def _wait_ice_complete(pc):
    if pc.iceGatheringState == "complete":
        return
    done = asyncio.Event()

    @pc.on("icegatheringstatechange")
    def _():
        if pc.iceGatheringState == "complete":
            done.set()

    try:
        await asyncio.wait_for(done.wait(), timeout=5)
    except asyncio.TimeoutError:
        pass  # proceed with whatever candidates we have


def _post_offer(server: str, sdp: str, sdp_type: str) -> dict:
    body = json.dumps({"sdp": sdp, "type": sdp_type}).encode()
    req = urllib.request.Request(
        server.rstrip("/") + "/api/offer",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=15) as resp:
        return json.loads(resp.read().decode())


# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------
async def run_benchmark(args):
    from aiortc import RTCIceServer

    ice = [RTCIceServer(urls=args.stun)]
    if args.turn:
        ice.append(RTCIceServer(urls=args.turn, username=args.turn_user,
                                credential=args.turn_pass))

    try:
        dur = wav_duration_secs(args.wav)
    except FileNotFoundError:
        print(f"ERROR: wav not found: {args.wav}\n"
              f"Run: python bench.py --make-sample  (placeholder), or drop a real recording there.")
        return 2

    print(f"server={args.server}  wav={args.wav} ({dur:.2f}s)  turns={args.n}  "
          f"stun={args.stun}  turn={'yes' if args.turn else 'no'}")
    print("measuring voice-to-voice latency (end-of-speech -> first bot audio)...\n")

    latencies = []
    for i in range(args.n):
        lat = await run_one_turn(
            args.server, args.wav, ice, args.rms_threshold,
            args.response_timeout, args.settle,
        )
        if lat is None:
            print(f"  turn {i + 1:>2}: TIMEOUT / no response")
        else:
            latencies.append(lat)
            print(f"  turn {i + 1:>2}: {lat * 1000:7.0f} ms")
        await asyncio.sleep(args.gap)

    print()
    if not latencies:
        print("No successful turns. Check: server running? API keys set? "
              "GREETING_ENABLED=false? real speech in the wav?")
        return 1

    _report(latencies, args.n)
    return 0


def _report(latencies, attempted):
    ms = [x * 1000 for x in latencies]
    print("voice-to-voice latency")
    print(f"  samples : {len(ms)}/{attempted} successful")
    print(f"  min     : {min(ms):7.0f} ms")
    print(f"  P50     : {percentile(ms, 50):7.0f} ms")
    print(f"  P95     : {percentile(ms, 95):7.0f} ms")
    print(f"  max     : {max(ms):7.0f} ms")
    if len(ms) > 1:
        print(f"  mean    : {statistics.mean(ms):7.0f} ms  (stdev {statistics.pstdev(ms):.0f})")
    print("\nPer-stage TTFB (STT/LLM/TTS): grep the SERVER logs for 'TTFB' "
          "(PipelineParams enable_metrics=True).")


def self_test():
    """Offline check of harness logic: percentiles, wav duration, RMS -- no network."""
    ok = True

    p = percentile([100, 200, 300, 400], 50)
    assert 240 <= p <= 260, p
    assert percentile([5], 95) == 5
    print("[ok] percentile math")

    import os
    import tempfile

    tmp = os.path.join(tempfile.gettempdir(), "kaira_bench_selftest.wav")
    make_placeholder_wav(tmp, seconds=2.0)
    d = wav_duration_secs(tmp)
    assert abs(d - 2.0) < 0.05, d
    print(f"[ok] wav duration = {d:.3f}s")

    try:
        import numpy as np  # noqa
        # fake an int16 frame-like object
        class _F:
            def to_ndarray(self):
                import numpy as np
                return (np.ones((1, 480)) * 3000).astype("int16")
        r = frame_rms(_F())
        assert 2900 < r < 3100, r
        print(f"[ok] frame RMS = {r:.0f}")
    except ImportError:
        ok = False
        print("[warn] numpy not importable -- RMS path untested")

    try:
        import aiortc  # noqa
        print(f"[ok] aiortc importable ({aiortc.__version__})")
    except Exception as e:
        ok = False
        print(f"[warn] aiortc not importable: {e}")

    print("\nself-test:", "PASS" if ok else "PARTIAL")
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser(description="Kaira voice-to-voice latency benchmark")
    ap.add_argument("--server", default="http://localhost:7860", help="SmallWebRTC server base URL")
    ap.add_argument("--wav", default=DEFAULT_WAV, help="pre-recorded ID+EN utterance (16kHz mono wav)")
    ap.add_argument("-n", type=int, default=10, help="number of turns to measure")
    ap.add_argument("--gap", type=float, default=1.5, help="seconds between turns")
    ap.add_argument("--settle", type=float, default=0.3,
                    help="grace period after utterance end before counting bot audio")
    ap.add_argument("--response-timeout", type=float, default=15.0, help="per-turn response timeout (s)")
    ap.add_argument("--rms-threshold", type=float, default=500.0, help="int16 RMS energy = 'bot is talking'")
    ap.add_argument("--stun", default="stun:stun.l.google.com:19302")
    ap.add_argument("--turn", default=None, help="turn:host:port?transport=udp")
    ap.add_argument("--turn-user", default=None)
    ap.add_argument("--turn-pass", default=None)
    ap.add_argument("--make-sample", action="store_true", help="write a placeholder wav and exit")
    ap.add_argument("--self-test", action="store_true", help="offline harness check and exit")
    ap.add_argument("--tc-help", action="store_true", help="print tc netem 3G recipe and exit")
    args = ap.parse_args()

    if args.tc_help:
        print(TC_HELP)
        return 0
    if args.make_sample:
        make_placeholder_wav(args.wav)
        return 0
    if args.self_test:
        return self_test()
    return asyncio.run(run_benchmark(args))


if __name__ == "__main__":
    sys.exit(main())
