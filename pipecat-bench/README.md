# Kaira Pipecat voice-agent latency benchmark

A self-hosted [Pipecat](https://pipecat.ai) voice pipeline plus a real WebRTC benchmark
client, built to measure **voice-to-voice latency** for Kaira (a realtime voice agent for
Indonesia, Bahasa Indonesia + English code-switching) and to see how that latency
**degrades on poor 3G**. The point is to compare a *server-realtime* architecture against
the *on-device* architecture (sherpa-onnx STT/TTS) being measured in parallel.

- **Transport:** self-hosted `SmallWebRTCTransport` — peer-to-peer, aiortc-based, no
  Daily / no paid relay. A WebSocket path is available for comparison (via the dev runner).
- **Pipeline:** `audio-in → STT → LLM → TTS → audio-out` (cascade), or a full-duplex
  speech-to-speech model (Gemini Live / OpenAI Realtime). VAD (Silero) + barge-in enabled.
- Verified against **pipecat-ai 1.5.0** (July 2026). Every class/import in `server.py`
  was checked against the installed package, not memory.

```
pipecat-bench/
  server.py        # the Pipecat pipeline + SmallWebRTC server (and dev-runner entrypoint)
  bench.py         # aiortc WebRTC client that measures voice-to-voice latency (P50/P95)
  smoke_test.py    # offline proof the scaffold runs (no keys/network)
  requirements.txt
  .env.example
  audio/sample_id_en.wav   # PLACEHOLDER noise -- replace with a real ID+EN recording
```

---

## What you must provide to run a real benchmark

**Accounts / API keys** (only for the providers you select in `.env`):

| Purpose | Default provider | Env var | Get it at |
|---|---|---|---|
| STT | Deepgram | `DEEPGRAM_API_KEY` | console.deepgram.com |
| LLM | OpenAI (gpt-4o) | `OPENAI_API_KEY` | platform.openai.com |
| TTS | Cartesia | `CARTESIA_API_KEY` | play.cartesia.ai |
| s2s (alt) | Google Gemini Live | `GOOGLE_API_KEY` | aistudio.google.com/apikey |
| s2s (alt) | OpenAI Realtime | `OPENAI_API_KEY` | platform.openai.com |
| TTS (ID) | ElevenLabs | `ELEVENLABS_API_KEY` | elevenlabs.io |
| STT (ID) | Google Cloud STT | `GOOGLE_APPLICATION_CREDENTIALS_JSON` | Google Cloud |
| STT (alt) | AssemblyAI | `ASSEMBLYAI_API_KEY` | assemblyai.com |

The default cascade needs **three keys: Deepgram + OpenAI + Cartesia**. The recommended
first run for Kaira (see below) is **Gemini Live s2s**, which needs **one key: Google**.

**Other requirements**

- **Python 3.11+** (pipecat is `requires-python >=3.11`; macOS system Python 3.9 will not work).
- A **real Bahasa-Indonesia + English recording** at `audio/sample_id_en.wav` (16 kHz
  mono). The committed file is placeholder noise so the harness runs; it will not transcribe.
- For real mobile-network numbers: a **TURN server** (see caveats) and a **deploy near
  Indonesia** (see below).

---

## Recommended deploy region

Deploy **as close to Indonesia as possible** — the existing Kaira backend is on fly.io, so:

- **fly.io `sin` (Singapore)** — first choice, matches current infra. ~30–60 ms RTT to
  most of Indonesia.
- Alternatives: AWS `ap-southeast-1` (Singapore) / `ap-southeast-3` (Jakarta), GCP
  `asia-southeast1` (Singapore) / `asia-southeast2` (Jakarta). Jakarta regions cut RTT
  further but Singapore is where the LLM/STT/TTS vendors' own endpoints usually live, so
  the *provider* leg often dominates — benchmark before optimizing region.

Latency is only meaningful measured **on the path to Indonesia**: deploy to `sin`, then
run `bench.py` from a device/VM actually in Indonesia (or shape the link — see 3G section).

---

## Run it locally

```bash
uv venv --python 3.11 .venv && source .venv/bin/activate   # or: python3.11 -m venv .venv
uv pip install -r requirements.txt                          # or: pip install -r requirements.txt
cp .env.example .env        # then fill in your keys

# offline sanity check -- no keys, no network:
python smoke_test.py

# start the server (greeting off so the only bot audio is the RESPONSE):
GREETING_ENABLED=false python server.py
# open http://localhost:7860  -> talk to the bot in the browser test UI
```

WebSocket transport for comparison (dev runner; webrtc is still default):

```bash
python server.py --runner        # then a client POSTs /start {"transport":"twilio"|"webrtc"}
```

## Run the benchmark

```bash
# (server running with GREETING_ENABLED=false)
python bench.py --server http://localhost:7860 --wav audio/sample_id_en.wav -n 10
```

Output is **voice-to-voice latency** — from the end of the user utterance to the first bot
audio frame — reported as min / **P50** / **P95** / max. This interval includes VAD
end-of-speech detection + STT + LLM + TTS + transport buffering + network, i.e. exactly
what the user waits through.

Helper commands:

```bash
python bench.py --self-test     # offline harness check
python bench.py --make-sample   # (re)write the placeholder wav
python bench.py --tc-help       # print the 3G netem recipe
```

**Per-stage TTFB** (STT vs LLM vs TTS): the server has `enable_metrics=True`, so it logs
per-service TTFB/processing time. `grep TTFB` in the server output to attribute latency to
a stage. A plain-WebRTC client can't see per-stage numbers, so this stays server-side.

---

## Network degradation (simulate poor 3G)

`bench.py --tc-help` prints the full recipe. Summary: use Linux `tc netem` on a host **on
the path** to the server (ideally the `sin` server's egress NIC, or a Linux router the
client traffic crosses). A 3G-ish profile:

```bash
sudo tc qdisc add dev eth0 root netem delay 300ms 50ms distribution normal loss 2%
# bad/congested 3G:
sudo tc qdisc change dev eth0 root netem delay 500ms 100ms loss 5% reorder 5% 50%
sudo tc qdisc del dev eth0 root          # remove
```

Shaping your Mac's loopback proves nothing — WebRTC's jitter buffer and loss recovery only
behave realistically over a real lossy path. **The faithful test is: deploy to `sin`, run
`bench.py` from Indonesia.** `tc netem` is a stand-in for when you can't.

---

## Known caveats (read before trusting the "serverless" story)

- **SmallWebRTC P2P needs NAT traversal.** On localhost it "just works." On real mobile
  networks the peers can't see each other directly — you need at minimum a **STUN** server
  (default: Google's public STUN) and, very often, a **TURN relay**.
- **Indonesian 3G is largely carrier-grade NAT (CGNAT).** Behind CGNAT, STUN frequently
  fails and the media path **must fall back to a TURN relay**. That means the "no relay
  infra" benefit of SmallWebRTC **evaporates for exactly the users this benchmark targets**
  — you end up hosting (and paying for / bearing the latency of) a TURN server anyway. Set
  `TURN_URL`/`TURN_USERNAME`/`TURN_PASSWORD` (e.g. self-hosted coturn in `sin`) and
  **re-measure**: TURN adds a relay hop, so P50/P95 over TURN is the honest mobile number,
  not the STUN/localhost number. Budget for this; don't report localhost latency as if it
  were field latency.
- **A TURN hop can be slower than a plain WebSocket** to the same server, so the WebSocket
  comparison path exists for a reason — measure both.
- The placeholder wav does not transcribe; real numbers require a real recording and live
  API keys (so the benchmark costs real provider spend per run).

---

## Speech-to-speech vs cascaded — which to benchmark first

| | Cascade (STT→LLM→TTS) | Speech-to-speech (Gemini Live / OpenAI Realtime) |
|---|---|---|
| Latency | Sum of 3 network hops + TTS TTFB; typically higher | Single full-duplex stream; usually **lower** |
| ID+EN code-switching | Depends on STT: **Deepgram nova-3 `multi` does NOT cover Indonesian**; needs Google STT or nova-2 `id` (mono) | Gemini Live handles Indonesian + code-switching **natively and well** |
| Control / swappability | Full: pick best STT, LLM, TTS independently; easy to log per-stage | Opaque single vendor; less tuning |
| TTS voice for Indonesian | ElevenLabs `eleven_multilingual_v2` or Google speak ID; Cartesia/Deepgram are EN-first | Model's built-in voices |
| Cost model | 3 metered services | 1 metered service (audio in+out) |

**Recommendation — benchmark Gemini Live (s2s) first.** For Kaira's ID+EN target it is the
single strongest fit: lowest expected latency, native Indonesian + code-switching, and one
API key. Set `MODE=s2s S2S_PROVIDER=gemini` and provide `GOOGLE_API_KEY`. Then benchmark
the **cascade** as the swappable/observable baseline — best config for ID+EN is
`STT_PROVIDER=google` (Chirp/`id-ID,en-US`) + `OPENAI_MODEL=gpt-4o` + `TTS_PROVIDER=elevenlabs`
(multilingual). Compare both against the on-device sherpa-onnx numbers. Note the default
cascade ships with **Deepgram STT** for out-of-the-box latency + the easy smoke test, but
Deepgram's multilingual model is English-centric — switch STT to Google for Indonesian.

---

## What a Rust/Slint client integration would require (not built here)

The Kaira client is Rust/Slint on Android (floor: Android 8.1 / WebView 68). This repo
only builds the **server + a Python/CLI benchmark client**. To connect the real client:

- **`webrtc-rs`** (pure-Rust WebRTC) is the natural fit: implement the SmallWebRTC signaling
  the server expects — `POST /api/offer {sdp,type[,pc_id]}` → `{sdp,type,pc_id}` — then feed
  mic audio in and play the answer track. No WebView needed, which sidesteps the WebView-68
  floor entirely (the browser prebuilt UI is dev-only). This is the recommended path.
- **Pipecat C++ client** (`pipecat-client-cxx`) over FFI is the alternative: it speaks the
  same transport, but adds a C++ toolchain + JNI/FFI bridge to Rust and a heavier build.
- Either way the client also needs the **ICE story** above (STUN + TURN creds), and on
  Android must handle mic permissions, audio focus, and Opus. The WebView/Dioxus path is a
  dead end on the old-WebView Android floor — go native (`webrtc-rs`).

---

## Verification status

- pipecat-ai 1.5.0 installed in a Python 3.11 venv; all 18 key imports resolve.
- `smoke_test.py` **passes**: `server.py` imports, and a mock cascade pipeline runs
  end-to-end through the real Pipecat runtime and emits TTS audio — no keys, no network.
- Standalone SmallWebRTC server boots, serves the prebuilt UI (`/client/`), redirects `/`,
  and exposes `/api/offer`.
- `bench.py --self-test` **passes** (percentile math, wav duration, RMS, aiortc import).
- Not exercised (needs live keys + a real recording + a real network path): an actual
  voice-to-voice turn and the resulting P50/P95 numbers.
