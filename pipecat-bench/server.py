#!/usr/bin/env python3
"""
Kaira Pipecat voice-agent benchmark server.

A self-hosted realtime voice pipeline:  audio-in -> STT -> LLM -> TTS -> audio-out
(cascaded), or a full-duplex speech-to-speech model (Gemini Live / OpenAI Realtime).

Transport: self-hosted SmallWebRTCTransport (peer-to-peer, aiortc-based). No Daily /
no paid relay. STUN is used for NAT traversal; an optional TURN relay can be supplied
via env for carrier-grade-NAT mobile networks (see README -- this matters a lot on
Indonesian 3G).

Verified against pipecat-ai==1.5.0 (July 2026). All third-party provider imports are
done lazily inside factory functions so this module imports cleanly even when only a
subset of provider extras is installed.

Run (standalone SmallWebRTC P2P server, serves a test UI at /):
    python server.py
    # open http://localhost:7860 in a browser and talk to the bot

Run via the Pipecat dev runner instead (adds a WebSocket/telephony path for
comparison, webrtc is still the default transport):
    python server.py --runner            # then POST /start with {"transport": "..."}

Everything is configured through environment variables -- see .env.example.
"""

import argparse
import asyncio
import os
import sys
from contextlib import asynccontextmanager

from dotenv import load_dotenv
from loguru import logger

# --- Pipecat core (stable across the cascade & s2s pipelines) ---------------
from pipecat.audio.vad.silero import SileroVADAnalyzer
from pipecat.frames.frames import LLMRunFrame
from pipecat.pipeline.pipeline import Pipeline
from pipecat.pipeline.worker import PipelineParams, PipelineWorker
from pipecat.processors.aggregators.llm_context import LLMContext
from pipecat.processors.aggregators.llm_response_universal import (
    LLMContextAggregatorPair,
    LLMUserAggregatorParams,
)
from pipecat.transports.base_transport import BaseTransport, TransportParams
from pipecat.workers.runner import WorkerRunner

load_dotenv(override=True)

# ---------------------------------------------------------------------------
# Configuration (all via env -- see .env.example)
# ---------------------------------------------------------------------------
MODE = os.getenv("MODE", "cascade").lower()              # "cascade" | "s2s"
STT_PROVIDER = os.getenv("STT_PROVIDER", "deepgram").lower()
LLM_PROVIDER = os.getenv("LLM_PROVIDER", "openai").lower()
TTS_PROVIDER = os.getenv("TTS_PROVIDER", "cartesia").lower()
S2S_PROVIDER = os.getenv("S2S_PROVIDER", "gemini").lower()  # "gemini" | "openai"

# Kaira targets Bahasa Indonesia + English code-switching.
SYSTEM_INSTRUCTION = os.getenv(
    "SYSTEM_INSTRUCTION",
    "You are Kaira, a warm and concise voice assistant for users in Indonesia. "
    "Users speak a natural mix of Bahasa Indonesia and English (code-switching); "
    "reply in whichever language(s) the user used, matching their mix. Your replies "
    "are spoken aloud, so avoid emojis, bullet points, markdown, or anything that "
    "cannot be read out loud. Keep answers short and helpful.",
)

# Greeting: the bot introduces itself on connect. Disable for clean bench runs so the
# only bot audio is the *response* to the benchmark utterance.
GREETING_ENABLED = os.getenv("GREETING_ENABLED", "true").lower() in ("1", "true", "yes")

HOST = os.getenv("HOST", "0.0.0.0")
PORT = int(os.getenv("PORT", "7860"))


# ---------------------------------------------------------------------------
# Service factories (lazy imports: only the chosen providers must be installed)
# ---------------------------------------------------------------------------
def create_stt():
    """Speech-to-text. Default Deepgram; Google/AssemblyAI for stronger Indonesian."""
    if STT_PROVIDER == "deepgram":
        from pipecat.services.deepgram.stt import DeepgramSTTService

        # nova-3 "multi" gives code-switching but its language set is EN-centric and
        # does NOT include Indonesian. For ID-heavy audio set STT_PROVIDER=google or
        # use DEEPGRAM_MODEL=nova-2 with DEEPGRAM_LANGUAGE=id (mono, no code-switch).
        return DeepgramSTTService(
            api_key=os.environ["DEEPGRAM_API_KEY"],
            settings=DeepgramSTTService.Settings(
                model=os.getenv("DEEPGRAM_MODEL", "nova-3"),
                language=os.getenv("DEEPGRAM_LANGUAGE", "multi"),
            ),
        )
    if STT_PROVIDER == "google":
        # Best Indonesian + code-switching of the cascade STT options. Needs pipecat-ai[google].
        from pipecat.services.google.stt import GoogleSTTService

        return GoogleSTTService(
            credentials=os.getenv("GOOGLE_APPLICATION_CREDENTIALS_JSON"),
            settings=GoogleSTTService.Settings(
                language_codes=os.getenv("GOOGLE_STT_LANGUAGES", "id-ID,en-US").split(","),
                model=os.getenv("GOOGLE_STT_MODEL", "chirp_2"),
            ),
        )
    if STT_PROVIDER == "assemblyai":
        from pipecat.services.assemblyai.stt import AssemblyAISTTService

        return AssemblyAISTTService(api_key=os.environ["ASSEMBLYAI_API_KEY"])
    raise ValueError(f"Unknown STT_PROVIDER={STT_PROVIDER!r}")


def create_llm():
    """Text LLM for the cascade path. Default OpenAI gpt-4o (strong ID+EN)."""
    if LLM_PROVIDER == "openai":
        from pipecat.services.openai.llm import OpenAILLMService

        return OpenAILLMService(
            api_key=os.environ["OPENAI_API_KEY"],
            settings=OpenAILLMService.Settings(
                model=os.getenv("OPENAI_MODEL", "gpt-4o"),
                system_instruction=SYSTEM_INSTRUCTION,
            ),
        )
    if LLM_PROVIDER == "google":
        from pipecat.services.google.llm import GoogleLLMService

        return GoogleLLMService(
            api_key=os.environ["GOOGLE_API_KEY"],
            settings=GoogleLLMService.Settings(
                model=os.getenv("GOOGLE_MODEL", "gemini-2.0-flash"),
                system_instruction=SYSTEM_INSTRUCTION,
            ),
        )
    raise ValueError(f"Unknown LLM_PROVIDER={LLM_PROVIDER!r}")


def create_tts():
    """Text-to-speech. Default Cartesia (low latency). ElevenLabs/Google speak ID better."""
    if TTS_PROVIDER == "cartesia":
        return _make_cartesia_tts()
    if TTS_PROVIDER == "elevenlabs":
        from pipecat.services.elevenlabs.tts import ElevenLabsTTSService

        return ElevenLabsTTSService(
            api_key=os.environ["ELEVENLABS_API_KEY"],
            settings=ElevenLabsTTSService.Settings(
                voice=os.getenv("ELEVENLABS_VOICE_ID", "21m00Tcm4TlvDq8ikWAM"),
                model=os.getenv("ELEVENLABS_MODEL", "eleven_multilingual_v2"),  # speaks Indonesian
            ),
        )
    if TTS_PROVIDER == "google":
        from pipecat.services.google.tts import GoogleTTSService

        return GoogleTTSService(
            credentials=os.getenv("GOOGLE_APPLICATION_CREDENTIALS_JSON"),
            settings=GoogleTTSService.Settings(
                voice=os.getenv("GOOGLE_TTS_VOICE", "id-ID-Chirp3-HD-Aoede"),
            ),
        )
    if TTS_PROVIDER == "deepgram":
        from pipecat.services.deepgram.tts import DeepgramTTSService

        return DeepgramTTSService(
            api_key=os.environ["DEEPGRAM_API_KEY"],
            settings=DeepgramTTSService.Settings(
                voice=os.getenv("DEEPGRAM_TTS_VOICE", "aura-2-andromeda-en"),  # English only
            ),
        )
    raise ValueError(f"Unknown TTS_PROVIDER={TTS_PROVIDER!r}")


def _make_cartesia_tts():
    from pipecat.services.cartesia.tts import CartesiaTTSService

    return CartesiaTTSService(
        api_key=os.environ["CARTESIA_API_KEY"],
        settings=CartesiaTTSService.Settings(
            voice=os.getenv("CARTESIA_VOICE_ID", "71a7ad14-091c-4e8e-a314-022ece01c121"),
            model=os.getenv("CARTESIA_MODEL", "sonic-2"),
            language=os.getenv("CARTESIA_LANGUAGE", "en"),
        ),
    )


def create_s2s_llm():
    """Full-duplex speech-to-speech model (does STT+reasoning+TTS in one service)."""
    if S2S_PROVIDER == "gemini":
        # Gemini Live handles Indonesian and ID<->EN code-switching well. Needs pipecat-ai[google].
        from pipecat.services.google.gemini_live.llm import GeminiLiveLLMService

        return GeminiLiveLLMService(
            api_key=os.environ["GOOGLE_API_KEY"],
            settings=GeminiLiveLLMService.Settings(
                system_instruction=SYSTEM_INSTRUCTION,
                voice=os.getenv("GEMINI_VOICE", "Aoede"),  # Puck, Charon, Kore, Fenrir, Aoede
            ),
        )
    if S2S_PROVIDER == "openai":
        from pipecat.services.openai.realtime.llm import OpenAIRealtimeLLMService

        return OpenAIRealtimeLLMService(
            api_key=os.environ["OPENAI_API_KEY"],
            settings=OpenAIRealtimeLLMService.Settings(
                system_instruction=SYSTEM_INSTRUCTION,
            ),
        )
    raise ValueError(f"Unknown S2S_PROVIDER={S2S_PROVIDER!r}")


# ---------------------------------------------------------------------------
# Pipeline assembly
# ---------------------------------------------------------------------------
def build_worker(transport: BaseTransport):
    """Build the pipeline + worker for the configured MODE. Returns (worker, context)."""
    context = LLMContext()

    if MODE == "s2s":
        llm = create_s2s_llm()
        # Silero VAD supplies local turn frames (barge-in) alongside the model's own
        # server-side VAD. This is what makes interruption feel responsive.
        user_agg, assistant_agg = LLMContextAggregatorPair(
            context,
            user_params=LLMUserAggregatorParams(vad_analyzer=SileroVADAnalyzer()),
        )
        pipeline = Pipeline(
            [
                transport.input(),
                user_agg,
                llm,               # speech in -> speech out
                transport.output(),
                assistant_agg,
            ]
        )
        logger.info(f"MODE=s2s provider={S2S_PROVIDER}")
    else:  # cascade
        stt = create_stt()
        llm = create_llm()
        tts = create_tts()
        # VAD lives in the user aggregator; interruption/barge-in is enabled by its
        # presence (Pipecat 1.x has no separate allow_interruptions flag).
        user_agg, assistant_agg = LLMContextAggregatorPair(
            context,
            user_params=LLMUserAggregatorParams(vad_analyzer=SileroVADAnalyzer()),
        )
        pipeline = Pipeline(
            [
                transport.input(),
                stt,
                user_agg,
                llm,
                tts,
                transport.output(),
                assistant_agg,
            ]
        )
        logger.info(
            f"MODE=cascade stt={STT_PROVIDER} llm={LLM_PROVIDER} tts={TTS_PROVIDER}"
        )

    worker = PipelineWorker(
        pipeline,
        params=PipelineParams(
            enable_metrics=True,        # per-service TTFB/processing metrics -> logs
            enable_usage_metrics=True,
        ),
    )
    return worker, context


def wire_conversation(transport: BaseTransport, worker: PipelineWorker, context: LLMContext):
    """Attach connect/disconnect handlers that start & stop the conversation."""

    @transport.event_handler("on_client_connected")
    async def _on_connected(transport, client):
        logger.info("Client connected")
        if GREETING_ENABLED:
            context.add_message(
                {"role": "developer", "content": "Please introduce yourself to the user."}
            )
            await worker.queue_frames([LLMRunFrame()])

    @transport.event_handler("on_client_disconnected")
    async def _on_disconnected(transport, client):
        logger.info("Client disconnected")
        await worker.cancel()


# ===========================================================================
# Standalone SmallWebRTC server (default entrypoint) -- explicit STUN/TURN control
# ===========================================================================
def build_ice_servers():
    """ICE config from env. STUN is enough behind normal NAT; TURN is required on
    carrier-grade NAT (common on Indonesian mobile) -- see README caveats."""
    from pipecat.transports.smallwebrtc.connection import IceServer

    servers = [IceServer(urls=os.getenv("STUN_URL", "stun:stun.l.google.com:19302"))]
    turn_url = os.getenv("TURN_URL")
    if turn_url:
        servers.append(
            IceServer(
                urls=turn_url,
                username=os.getenv("TURN_USERNAME"),
                credential=os.getenv("TURN_PASSWORD"),
            )
        )
        logger.info(f"TURN relay configured: {turn_url}")
    else:
        logger.warning(
            "No TURN_URL set -- P2P will fail behind carrier-grade NAT (Indonesian 3G). "
            "See README."
        )
    return servers


def run_standalone_server():
    import uvicorn
    from fastapi import BackgroundTasks, FastAPI
    from fastapi.responses import RedirectResponse
    from pipecat_ai_small_webrtc_prebuilt.frontend import SmallWebRTCPrebuiltUI

    from pipecat.transports.smallwebrtc.connection import SmallWebRTCConnection
    from pipecat.transports.smallwebrtc.transport import SmallWebRTCTransport

    ice_servers = build_ice_servers()
    pcs_map: dict[str, SmallWebRTCConnection] = {}

    @asynccontextmanager
    async def lifespan(app: FastAPI):
        yield
        await asyncio.gather(*(pc.disconnect() for pc in pcs_map.values()))
        pcs_map.clear()

    app = FastAPI(lifespan=lifespan)
    app.mount("/client", SmallWebRTCPrebuiltUI)

    @app.get("/", include_in_schema=False)
    async def _root():
        return RedirectResponse(url="/client/")

    async def _run_bot(webrtc_connection: SmallWebRTCConnection):
        transport = SmallWebRTCTransport(
            webrtc_connection=webrtc_connection,
            params=TransportParams(audio_in_enabled=True, audio_out_enabled=True),
        )
        worker, context = build_worker(transport)
        wire_conversation(transport, worker, context)
        runner = WorkerRunner(handle_sigint=False)
        await runner.add_workers(worker)
        await runner.run()

    @app.post("/api/offer")
    async def _offer(request: dict, background_tasks: BackgroundTasks):
        pc_id = request.get("pc_id")
        if pc_id and pc_id in pcs_map:
            conn = pcs_map[pc_id]
            await conn.renegotiate(
                sdp=request["sdp"],
                type=request["type"],
                restart_pc=request.get("restart_pc", False),
            )
        else:
            conn = SmallWebRTCConnection(ice_servers)
            await conn.initialize(sdp=request["sdp"], type=request["type"])

            @conn.event_handler("closed")
            async def _closed(c: SmallWebRTCConnection):
                pcs_map.pop(c.pc_id, None)

            background_tasks.add_task(_run_bot, conn)

        answer = conn.get_answer()
        pcs_map[answer["pc_id"]] = conn
        return answer

    logger.info(f"SmallWebRTC server on http://{HOST}:{PORT}  (open http://localhost:{PORT})")
    uvicorn.run(app, host=HOST, port=PORT)


# ===========================================================================
# Pipecat dev-runner entrypoint (adds WebSocket/telephony path for comparison)
# ===========================================================================
async def bot(runner_args):
    """Entry point for the Pipecat dev runner (`python server.py --runner`).

    Provides webrtc (SmallWebRTC P2P, default) AND a websocket/telephony transport so
    you can benchmark the same pipeline over both transports for comparison.
    """
    from pipecat.runner.utils import create_transport
    from pipecat.transports.websocket.fastapi import FastAPIWebsocketParams

    transport_params = {
        "webrtc": lambda: TransportParams(audio_in_enabled=True, audio_out_enabled=True),
        "twilio": lambda: FastAPIWebsocketParams(audio_in_enabled=True, audio_out_enabled=True),
        "telnyx": lambda: FastAPIWebsocketParams(audio_in_enabled=True, audio_out_enabled=True),
    }
    transport = await create_transport(runner_args, transport_params)
    worker, context = build_worker(transport)
    wire_conversation(transport, worker, context)
    runner = WorkerRunner(handle_sigint=getattr(runner_args, "handle_sigint", True))
    await runner.add_workers(worker)
    await runner.run()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Kaira Pipecat benchmark server")
    parser.add_argument(
        "--runner",
        action="store_true",
        help="Launch via the Pipecat dev runner (webrtc + websocket) instead of the "
        "standalone SmallWebRTC server.",
    )
    args, _ = parser.parse_known_args()

    if args.runner:
        sys.argv = [a for a in sys.argv if a != "--runner"]
        from pipecat.runner.run import main

        main()
    else:
        run_standalone_server()
