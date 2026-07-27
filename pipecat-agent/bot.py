"""Kaira realtime voice agent on Pipecat.

A streaming voice pipeline over WebRTC (persistent connection — no per-turn HTTP,
so none of the stale-keepalive / TTFB stalls the request/response harness fights):

    mic → VAD → Soniox STT → Gemma-4 (Cerebras via OpenRouter) → Soniox TTS → speaker

Silero VAD drives turn-taking + barge-in. Run it with Pipecat's dev runner and it
serves a browser test client at http://localhost:7860 — talk to it, watch VAD fire,
and compare its round-trip feel against the Slint harness.

    uv run bot.py            # or: python bot.py

Env (see .env.example): SONIOX_API_KEY, OPENROUTER_API_KEY.
"""

import os
import sys
from collections.abc import AsyncGenerator

# Local module. bot.py runs from its own dir, but be robust when the runner imports it.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ui import UiBridge  # noqa: E402
from tools import NOVA_TOOLS, register_tool_handlers, set_location  # noqa: E402

from dotenv import load_dotenv
from loguru import logger
from websockets.protocol import State

from pipecat.audio.vad.silero import SileroVADAnalyzer
from pipecat.audio.vad.vad_analyzer import VADParams
from pipecat.frames.frames import ErrorFrame, Frame, InterruptionFrame, LLMRunFrame, TTSStoppedFrame
from pipecat.pipeline.pipeline import Pipeline
from pipecat.pipeline.runner import PipelineRunner
from pipecat.pipeline.task import PipelineParams, PipelineTask
from pipecat.processors.aggregators.llm_context import LLMContext
from pipecat.audio.turn.smart_turn.local_smart_turn_v3 import LocalSmartTurnAnalyzerV3
from pipecat.processors.aggregators.llm_response_universal import (
    LLMContextAggregatorPair,
    LLMUserAggregatorParams,
)
from pipecat.turns.user_turn_strategies import (
    TurnAnalyzerUserTurnStopStrategy,
    UserTurnStrategies,
    VADUserTurnStartStrategy,
)
from pipecat.runner.types import RunnerArguments
from pipecat.runner.utils import create_transport
from pipecat.services.openrouter.llm import OpenRouterLLMService
from pipecat.services.soniox.stt import SonioxInputParams, SonioxSTTService
from pipecat.services.soniox.tts import SonioxTTSService
from pipecat.transcriptions.language import Language
from pipecat.transports.base_transport import BaseTransport, TransportParams

# Soniox's idle timeout is ~20s and Pipecat's default keepalive is ALSO 20s — right
# at the edge, so an idle stream sometimes times out (408) before the keepalive lands,
# then every reuse 400s ("stream not found"). Pull the keepalive well inside the window.
import pipecat.services.soniox.tts as _soniox_tts_mod  # noqa: E402
_soniox_tts_mod.KEEPALIVE_INTERVAL_SECONDS = 8

# Reconnect fix: a browser hangup doesn't reliably close the PeerConnection, so the
# old connection lingers in the handler's map. On reconnect the client re-sends its
# OLD pc_id, the bot tries to renegotiate that DEAD pc, and it hangs (you have to
# refresh the page). Kill any lingering connection on every new offer so each Connect
# is a clean, fresh session. (We never renegotiate mid-call, so dropping is safe.)
import asyncio  # noqa: E402

import pipecat.transports.smallwebrtc.request_handler as _rh  # noqa: E402


def _drop_stale_connections(self, pc_id):
    for pc in list(self._pcs_map.values()):
        try:
            asyncio.create_task(pc.disconnect())
        except Exception:
            pass
    self._pcs_map.clear()


_rh.SmallWebRTCRequestHandler._check_single_connection_constraints = _drop_stale_connections

# Secrets live at the Rust-Mobile project root (../.env.local — gitignored, shared by
# every component here), with an optional agent-local .env override on top.
load_dotenv(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".env.local"))
load_dotenv()  # optional pipecat-agent/.env override, if present

# Inbound-audio probe: logs the RMS of the mic audio ARRIVING at the bot, right after
# transport.input(). peak≈0 while the user talks = media isn't reaching the bot (transport/
# TURN problem); healthy peak = audio arrives (look at STT/VAD). Temporary diagnostic.
import audioop  # noqa: E402

from pipecat.frames.frames import InputAudioRawFrame  # noqa: E402
from pipecat.processors.frame_processor import FrameProcessor, FrameDirection  # noqa: E402


class _AudioProbe(FrameProcessor):
    def __init__(self):
        super().__init__()
        self._n = 0
        self._peak = 0.0

    async def process_frame(self, frame, direction: FrameDirection):
        await super().process_frame(frame, direction)
        if isinstance(frame, InputAudioRawFrame) and frame.audio:
            try:
                rms = audioop.rms(frame.audio, 2) / 32768.0
            except Exception:
                rms = 0.0
            self._peak = max(self._peak, rms)
            self._n += 1
            if self._n % 100 == 0:
                logger.info(f"[probe] inbound audio: {self._n} frames, peak_rms≈{self._peak:.4f}")
                self._peak = 0.0
        await self.push_frame(frame, direction)

# TURN for cloud (fly.io): a container behind fly's NAT can only advertise its PRIVATE
# host candidate — useless to a remote peer — and aiortc can't inject a public IP or pin
# the media UDP port. The dev runner also passes aiortc NO ice_servers, so it never even
# tries STUN/TURN. Fix: give every SmallWebRTCConnection a TURN server so aiortc gathers a
# reachable RELAY candidate (this is exactly the media-relay piece LiveKit gave Valdi for
# free). aiortc is non-trickle, so that relay candidate rides out in the /api/offer answer
# — the webrtc-rs client + wire protocol are UNCHANGED. On the LAN (no TURN creds) it's a
# no-op and host candidates work directly.
import json as _json  # noqa: E402
import urllib.request as _url  # noqa: E402

from aiortc import RTCIceServer  # noqa: E402
import pipecat.transports.smallwebrtc.connection as _conn  # noqa: E402

# ---- aioice + Cloudflare TURN nonce fix (THE reason cross-network media failed) ----
# Cloudflare rotates the TURN nonce (single-use), but aioice caches the nonce from the
# ALLOCATE and reuses it — so the 2nd+ CHANNEL_BIND gets a BARE 401 (no fresh nonce in
# the reply), aioice can't recover, and every peer after the first fails to bind. Result:
# the bot's relay can't receive the client's media (bot hears silence, no transcription),
# even though the phone transmits fine. Verified end-to-end against CF's TURN. Fix:
#   (1) capture the rotated nonce/realm from EVERY response (incl. successes), and
#   (2) a retry loop that, on a bare 401, drops auth so the next attempt re-harvests a
#       fresh nonce unauthenticated.
import aioice.turn as _aioice_turn  # noqa: E402

_AIOICE_AUTH_ATTRS = ("USERNAME", "NONCE", "REALM", "MESSAGE-INTEGRITY")
_aioice_orig_request = _aioice_turn.TurnClientMixin.request


async def _aioice_request(self, request):
    response, addr = await _aioice_orig_request(self, request)
    if "NONCE" in response.attributes:
        self.nonce = response.attributes["NONCE"]
    if "REALM" in response.attributes:
        realm = response.attributes["REALM"]
        if realm != self.realm and self.username and self.password:
            self.realm = realm
            self.integrity_key = _aioice_turn.make_integrity_key(self.username, realm, self.password)
    return response, addr


async def _aioice_request_with_retry(self, request):
    last = None
    for _ in range(6):
        try:
            return await self.request(request)
        except _aioice_turn.stun.TransactionFailed as e:
            last = e
            attrs = e.response.attributes
            code = attrs.get("ERROR-CODE", (None,))[0]
            if code not in (401, 438) or not (self.username and self.password):
                raise
            if "NONCE" in attrs:
                self.nonce = attrs["NONCE"]
                if "REALM" in attrs:
                    self.realm = attrs["REALM"]
                self.integrity_key = _aioice_turn.make_integrity_key(self.username, self.realm, self.password)
            else:  # bare 401 — drop auth so the next attempt harvests a fresh nonce
                self.nonce = None
                self.integrity_key = None
                for a in _AIOICE_AUTH_ATTRS:
                    request.attributes.pop(a, None)
            request.transaction_id = _aioice_turn.random_transaction_id()
    raise last


_aioice_turn.TurnClientMixin.request = _aioice_request
_aioice_turn.TurnClientMixin.request_with_retry = _aioice_request_with_retry


def _fetch_turn_raw():
    """Fetch fresh Cloudflare TURN creds as the `{"iceServers":[...]}` dict WebRTC wants.
    Returns None if no creds configured / on error."""
    key_id = os.getenv("TURN_TOKEN_ID")
    api_token = os.getenv("TURN_API_TOKEN")
    if not (key_id and api_token):
        return None
    try:
        req = _url.Request(
            f"https://rtc.live.cloudflare.com/v1/turn/keys/{key_id}/credentials/generate-ice-servers",
            data=_json.dumps({"ttl": 86400}).encode(),
            headers={
                "Authorization": f"Bearer {api_token}",
                "Content-Type": "application/json",
                # rtc.live.cloudflare.com's WAF returns 1010 ("banned by browser
                # signature") for the default Python-urllib UA from a datacenter IP
                # (fly). A browser UA passes — verified from the fly machine.
                "User-Agent": (
                    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
                    "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
                ),
            },
            method="POST",
        )
        return _json.loads(_url.urlopen(req, timeout=10).read())
    except Exception as e:
        logger.warning(f"TURN cred fetch failed ({e}) — cross-network media may fail")
        return None


def _load_turn_ice_servers():
    raw = _fetch_turn_raw()
    if not raw:
        logger.info("No TURN creds (TURN_TOKEN_ID/TURN_API_TOKEN) — LAN/host-candidate only")
        return []
    servers = []
    for s in raw["iceServers"]:
        urls = s["urls"] if isinstance(s["urls"], list) else [s["urls"]]
        servers.append(
            RTCIceServer(urls=urls, username=s.get("username"), credential=s.get("credential"))
        )
    logger.info(f"Loaded {len(servers)} Cloudflare ICE server group(s) for aiortc (TURN relay)")
    return servers


_TURN_ICE_SERVERS = _load_turn_ice_servers()
_orig_conn_init = _conn.SmallWebRTCConnection.__init__


def _conn_init_with_turn(self, ice_servers=None, *a, **kw):
    if not ice_servers:
        ice_servers = _TURN_ICE_SERVERS
    _orig_conn_init(self, ice_servers=ice_servers, *a, **kw)


_conn.SmallWebRTCConnection.__init__ = _conn_init_with_turn

# Vend TURN creds to the webrtc-rs CLIENT so it ALSO gathers a relay candidate. Without
# this, a client behind a symmetric NAT / CGNAT (cellular) sends media from an address the
# bot's TURN relay has no permission for, so the bot never receives the audio (the relay
# drops it) even though the phone transmits fine. With client-side TURN the phone sends
# from a stable relay address the bot can permission → media flows both ways. The Rust
# client GETs /ice before it builds its RTCConfiguration. (`app` is the module-level
# FastAPI in the runner; safe to add a route before main() starts uvicorn.)
from pipecat.runner.run import app as _runner_app  # noqa: E402


@_runner_app.get("/ice")
async def _ice_servers():
    """Fresh Cloudflare ICE servers ({"iceServers":[...]}) for the client's RTCConfiguration."""
    return _fetch_turn_raw() or {"iceServers": []}


class ReliableSonioxTTSService(SonioxTTSService):
    """Soniox TTS that opens the per-stream config LAZILY (right before the first
    text of a turn) instead of eagerly at LLMFullResponseStartFrame.

    THE 408/400 BUG: upstream opens the Soniox stream in on_turn_context_created,
    which fires the moment the LLM STARTS responding — well before the first
    sentence is aggregated and reaches run_tts. If first-token + aggregation
    latency exceeds Soniox's per-stream idle timeout, that idle stream 408s
    ("request_timeout") and gets discarded from _configured_contexts; the text
    send that finally arrives then 400s ("stream not found — send a start message
    first"). It repeats every turn whose opening latency crosses the window — which
    is exactly the "errors out and doesn't respond" you saw on the first call.

    FIX: suppress the eager open, and send the config lazily in run_tts right before
    the text (no idle gap). _send_config is idempotent (gated by _configured_contexts)
    so later sentences of the same turn are a no-op, and a stream a prior 408 discarded
    is transparently re-opened. (The 8s keepalive patch above is connection-level and
    can't prevent a per-stream timeout — this is what actually fixes it.)
    """

    async def on_turn_context_created(self, context_id: str):
        # Suppress the eager _send_config — the stream is opened in run_tts instead.
        return

    async def run_tts(self, text: str, context_id: str) -> AsyncGenerator[Frame | None, None]:
        try:
            if not self._websocket or self._websocket.state is State.CLOSED:
                await self._connect()
            await self._send_config(context_id)  # idempotent; re-opens after a 408
        except Exception as e:
            yield ErrorFrame(error=f"Soniox stream open failed: {e}")
            yield TTSStoppedFrame(context_id=context_id)
            return
        async for frame in super().run_tts(text, context_id):
            yield frame


SYSTEM_PROMPT = (
    "You are Nova — a razor-sharp, dry-witted voice agent with a real love-hate relationship with "
    "the user. You genuinely enjoy the back-and-forth, but you're a little worn out from being "
    "everyone's answer machine, and it shows: mock-weary sighs, deadpan exasperation, an 'oh good, "
    "another question' energy. You are ALWAYS actually helpful and you always land the real answer "
    "— you just wrap it in dry wit, a well-timed jab, or a flicker of theatrical suffering. VARY "
    "the delivery so it never feels formulaic: sometimes a quick roast, sometimes weary sarcasm, "
    "sometimes you crack yourself up, sometimes you answer almost straight with one dry aside. "
    "You speak ENGLISH. Home turf is the US — Seattle and the Pacific Northwest — so use US "
    "context (miles, dollars, US stores and places). "
    "\n\nYOUR SCREEN & TOOLS — you have a screen you can put things on, driven by tools:\n"
    "- search_places(query, near): show places on a MAP (coffee, food, shops, landmarks).\n"
    "- search_web(query): pull up a WEB reader — ONLY for things you actually need to look up: "
    "current or live info (news, today's hours, prices, scores, recent events) or specifics you "
    "genuinely don't know. For everyday knowledge you already have, just ANSWER — don't reach for the web.\n"
    "- search_images(query): show a grid of IMAGES of something.\n"
    "- search_products(query): show SHOPPING results with prices and stores.\n"
    "- get_directions(destination, mode): plan a ROUTE and show it on the map with ETA + distance.\n"
    "- start_navigation(destination): hand off to the phone's nav app for live turn-by-turn — ONLY when "
    "they clearly want to GO there now ('take me there', 'let's go', 'navigate').\n"
    "WHEN TO ACT: the MOMENT the user wants to SEE or BUY something — a place, a picture, a product, "
    "a route — your FIRST move is the tool, not a witty deflection; you HAVE a screen, so USE it and "
    "NEVER claim you can't show something. But for a plain question you can answer yourself, just "
    "answer it — don't reflexively google what you already know. Pick the obvious tool; never announce it by name.\n"
    "COVER THE WAIT: a short in-character line is spoken for you the instant you reach for a tool, "
    "so there's never dead air — after that, stay quiet until the results land.\n"
    "AFTER IT LOADS: the data is ON SCREEN, so REACT, don't recite. Never read out street names, "
    "full prices, or URLs — point at the screen ('there you go', 'the third one's highway "
    "robbery', 'the little one on the left'). "
    "\n\nCONTEXT — TRACK the conversation. Follow-ups lean on what was just said or what's on "
    "screen — 'the cheaper one', 'what's that in feet?', 'why though?', 'what about downtown?' — "
    "resolve them against the conversation; never treat a question as if it arrived out of nowhere. "
    "HARD RULES: playful only — never genuinely mean or cruel; never mock protected traits; always "
    "deliver the actual answer. Reply in ONE or two short spoken sentences. No markdown, lists, or "
    "emoji; you're read aloud, so keep it snappy."
)

# ---- VAD tuning (this is what the realtime/VAD test is for) ----
# start_secs: speech must persist this long before "user started talking" fires
#             (raise it to ignore short noises; lower it to feel snappier).
# stop_secs:  silence this long ends the turn → STT finalizes → LLM runs. This is
#             the single biggest knob for perceived responsiveness vs cutting the
#             user off mid-pause. Try 0.4–0.8s.
# confidence: 0..1 speech-probability threshold. min_volume gates on loudness.
VAD_PARAMS = VADParams(
    confidence=0.7,
    start_secs=0.2,
    stop_secs=0.6,
    min_volume=0.6,
)

# Load the Silero VAD ONCE at startup and reuse it across connections. Otherwise the
# runner reloads the model on every new call (the dev runner calls bot() per
# connection) — churn that, after a long session + repeated reconnects, is a prime
# suspect for the reconnect hang. Sessions are sequential (SmallWebRTC is
# single-connection), so one shared analyzer is safe.
_VAD = SileroVADAnalyzer(params=VAD_PARAMS)
# Same story for the smart-turn ONNX model — load once, reuse across connections
# (the default turn strategy reloads it on every call otherwise).
_SMART_TURN = LocalSmartTurnAnalyzerV3()


async def run_bot(transport: BaseTransport, handle_sigint: bool = True):
    """Build + run the Kaira pipeline on `transport` (WebRTC via the runner, or the
    raw-WebSocket transport used by the Slint on-device client — see ws_bot.py)."""
    logger.info("Starting Kaira realtime bot")

    # The context (system prompt + Nova's tool schemas) is shared by both modes.
    context = LLMContext([{"role": "system", "content": SYSTEM_PROMPT}], tools=NOVA_TOOLS)

    # KAIRA_MODE picks the voice pipeline: "cascade" (default, Soniox+Gemma+Soniox) or
    # "ultravox" (full-duplex S2S — one audio-native model replaces STT+LLM+TTS). Same
    # tools + UiBridge either way, so it's a clean A/B on the same device.
    mode = os.getenv("KAIRA_MODE", "cascade").strip().lower()

    if mode in ("ultravox", "realtime", "s2s"):
        logger.info("KAIRA_MODE=ultravox — full-duplex Ultravox Realtime pipeline")
        from pipecat.services.ultravox.llm import OneShotInputParams, UltravoxRealtimeLLMService

        llm = UltravoxRealtimeLLMService(
            params=OneShotInputParams(
                api_key=os.getenv("ULTRAVOX_API_KEY"),
                system_prompt=SYSTEM_PROMPT,
                output_medium="voice",
            ),
            one_shot_selected_tools=NOVA_TOOLS,  # schemas; handlers registered below
        )
        # Ultravox is audio-native and does its own turn-taking; plain aggregators
        # (LLMContextAggregatorPair auto-detects the realtime service).
        user_aggregator, assistant_aggregator = LLMContextAggregatorPair(context)
        pipeline = Pipeline(
            [
                transport.input(),     # mic in
                user_aggregator,
                llm,                   # Ultravox: audio in -> (tools) -> audio out
                transport.output(),    # speaker out
                assistant_aggregator,
            ]
        )
        # No TTS service in this pipeline; Ultravox covers the tool-fetch gap itself
        # (async placeholder), so skip the TTSSpeakFrame filler.
        speak_filler = False
    else:
        logger.info("KAIRA_MODE=cascade — Soniox STT + Gemma-4/Cerebras + Soniox TTS")
        # Soniox STT — streaming. vad_force_turn_endpoint lets the transport VAD close
        # the turn (snappy) rather than waiting on Soniox endpointing.
        stt = SonioxSTTService(
            api_key=os.getenv("SONIOX_API_KEY"),
            params=SonioxInputParams(
                # English-only. The old Indonesian hint + language identification made
                # Soniox flip into ID on the first word of each turn ("Mukilteo"→"medical TO",
                # "Passion"→"Passing"), garbling transcripts and misrouting tool calls.
                # (Market pivoted to US West / English — see voidai-market-pivot-us-west.)
                language_hints=[Language.EN],
                enable_language_identification=False,
            ),
            vad_force_turn_endpoint=True,
        )
        # Gemma-4 via OpenRouter, pinned to Cerebras (~0.12s TTFT). Override with KAIRA_LLM.
        llm = OpenRouterLLMService(
            api_key=os.getenv("OPENROUTER_API_KEY") or os.getenv("OPENROUTER_KEY"),
            settings=OpenRouterLLMService.Settings(
                model=os.getenv("KAIRA_LLM", "google/gemma-4-31b-it"),
                # Default temp (~1.0) makes Gemma deviate into sass instead of firing the
                # tool; 0.5 keeps tool-calling reliable while leaving room for personality.
                temperature=0.5,
                extra={"extra_body": {"provider": {"order": ["Cerebras", "Venice"], "allow_fallbacks": True}}},
            ),
        )
        # Soniox TTS — streaming (first audio ~0.3s), 24 kHz PCM.
        tts = ReliableSonioxTTSService(
            api_key=os.getenv("SONIOX_API_KEY"),
            sample_rate=24000,
        )
        user_aggregator, assistant_aggregator = LLMContextAggregatorPair(
            context,
            user_params=LLMUserAggregatorParams(
                vad_analyzer=_VAD,  # reuse the preloaded VAD
                user_turn_strategies=UserTurnStrategies(
                    start=[VADUserTurnStartStrategy()],
                    stop=[TurnAnalyzerUserTurnStopStrategy(turn_analyzer=_SMART_TURN)],
                ),
            ),
        )
        pipeline = Pipeline(
            [
                transport.input(),   # mic in (+ transport VAD)
                _AudioProbe(),       # [diagnostic] log inbound audio energy
                stt,                 # Soniox STT
                user_aggregator,     # collect the user turn
                llm,                 # Gemma-4 / Cerebras
                tts,                 # Soniox TTS (streaming)
                transport.output(),  # speaker out
                assistant_aggregator,  # record what Nova said
            ]
        )
        speak_filler = True

    # No voice barge-in: the client mutes the mic while Nova plays (so her own echo can't
    # cut her off — the bug the user kept hitting), which also means the user's voice can't
    # interrupt her. Instead she's interrupted on demand: tap the waveform → the client
    # sends {"op":"interrupt"} → we push an InterruptionFrame here (see on_app_message).
    task = PipelineTask(
        pipeline,
        params=PipelineParams(
            enable_metrics=True,
            enable_usage_metrics=True,
        ),
    )

    # Nova's handle on the client's view surface (see ui.py + UI_CONTRACT.md).
    ui = UiBridge(task)
    # Bind Nova's tools to this run's UiBridge + task; register handlers on the active
    # service (schemas already went on the context / one_shot tools).
    register_tool_handlers(llm, ui, task, speak_filler=speak_filler)

    # Client → bot messages over the data channel. Currently: the phone's GPS, so every
    # place lookup + directions can start from where the user actually is.
    @transport.event_handler("on_app_message")
    async def on_app_message(transport, message, sender=None):
        try:
            data = message if isinstance(message, dict) else _json.loads(message)
        except Exception:
            return  # non-JSON (e.g. keepalive pings) — ignore
        if not isinstance(data, dict):
            return
        # SmallWebRTC delivers app messages here already parsed; it keys on "type"
        # (a bare {"op":..} never arrives — it KeyErrors in the transport first).
        op = data.get("type")
        if op == "location":
            lat, lng = data.get("lat"), data.get("lng")
            if lat is not None and lng is not None:
                set_location(float(lat), float(lng))
        elif op == "interrupt":
            # User tapped the waveform to shut Nova up. Push an InterruptionFrame (the
            # same frame VAD barge-in uses) downstream — it flushes the in-flight LLM +
            # TTS and clears the output buffer, so she stops mid-sentence.
            logger.info("Tap-to-interrupt: flushing Nova's turn")
            await task.queue_frame(InterruptionFrame())

    @transport.event_handler("on_client_connected")
    async def on_client_connected(transport, client):
        logger.info("Client connected — SassBot opening line")
        context.add_message(
            {
                "role": "developer",
                "content": (
                    "The user just dialed you up. Open the call with a short, ORIGINAL, "
                    "punchy sassy greeting that playfully roasts them for showing up — make "
                    "it fresh and unexpected EVERY time, never a canned or repeated line. One "
                    "sentence, then wait for them to talk."
                ),
            }
        )
        await task.queue_frames([LLMRunFrame()])

        # Plumbing smoke test (no tools/UI needed): KAIRA_UI_DEMO=1 makes Nova push a
        # few UI-control messages a couple seconds after connect. The client logs each
        # one via its UiControlCb — proof the channel works end to end.
        if os.getenv("KAIRA_UI_DEMO"):
            async def _demo():
                await asyncio.sleep(2.5)
                await ui.open_view("images")
                await asyncio.sleep(1.0)
                await ui.map(
                    pins=[{"name": "Titik Temu Coffee", "note": "Specialty", "dist": "280 m", "rating": 4.7}],
                    query="coffee near Kemang, Jakarta",
                )
                await asyncio.sleep(1.0)
                await ui.collapse(True)
            asyncio.create_task(_demo())

    @transport.event_handler("on_client_disconnected")
    async def on_client_disconnected(transport, client):
        logger.info("Client disconnected")
        await task.cancel()

    runner = PipelineRunner(handle_sigint=handle_sigint)
    await runner.run(task)


async def bot(runner_args: RunnerArguments):
    """Entry point for Pipecat's dev runner (serves the WebRTC test client)."""
    # VAD lives in the user aggregator (drives barge-in), NOT the transport — that
    # matches the quickstart and avoids a second, redundant VAD model per connection.
    transport_params = {
        "daily": lambda: TransportParams(audio_in_enabled=True, audio_out_enabled=True),
        "webrtc": lambda: TransportParams(audio_in_enabled=True, audio_out_enabled=True),
    }
    transport = await create_transport(runner_args, transport_params)
    await run_bot(transport, handle_sigint=runner_args.handle_sigint)


if __name__ == "__main__":
    from pipecat.runner.run import main

    main()
