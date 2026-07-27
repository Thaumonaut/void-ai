#!/usr/bin/env python3
"""
Offline smoke test -- proves the scaffold is not broken WITHOUT any API keys or network.

It does two things:
  1. imports server.py  (all provider imports are lazy, so this just validates the
     module + pipecat import surface)
  2. builds a cascade-shaped pipeline out of MOCK STT/LLM/TTS frame processors and runs
     real audio-less frames through the real Pipeline runtime via pipecat's own
     run_test harness, asserting a TTS audio frame comes out the far end.

Run:  python smoke_test.py
"""

import asyncio
import sys

from pipecat.frames.frames import TextFrame, TranscriptionFrame, TTSAudioRawFrame
from pipecat.pipeline.pipeline import Pipeline
from pipecat.processors.frame_processor import FrameDirection, FrameProcessor
from pipecat.tests.utils import run_test


class MockLLM(FrameProcessor):
    """Stand-in for the LLM: a user transcript in -> a spoken reply (TextFrame) out."""

    async def process_frame(self, frame, direction: FrameDirection):
        await super().process_frame(frame, direction)
        if isinstance(frame, (TranscriptionFrame, TextFrame)) and direction == FrameDirection.DOWNSTREAM:
            await self.push_frame(TextFrame(text=f"reply to: {frame.text}"), direction)
        else:
            await self.push_frame(frame, direction)


class MockTTS(FrameProcessor):
    """Stand-in for TTS: a TextFrame in -> a TTSAudioRawFrame (fake PCM) out."""

    async def process_frame(self, frame, direction: FrameDirection):
        await super().process_frame(frame, direction)
        if isinstance(frame, TextFrame) and direction == FrameDirection.DOWNSTREAM:
            await self.push_frame(frame, direction)  # keep transcript visible
            await self.push_frame(
                TTSAudioRawFrame(audio=b"\x00\x00" * 160, sample_rate=16000, num_channels=1),
                direction,
            )
        else:
            await self.push_frame(frame, direction)


async def _run():
    print("[1/2] importing server.py (lazy provider imports -> no keys needed)...")
    import server  # noqa: F401

    assert hasattr(server, "build_worker"), "server.build_worker missing"
    assert hasattr(server, "bot"), "server.bot (runner entrypoint) missing"
    print("      server.py imported OK; build_worker + bot present")

    print("[2/2] running a mock cascade pipeline through the real Pipecat runtime...")
    pipeline = Pipeline([MockLLM(), MockTTS()])
    received_down, _ = await run_test(
        pipeline,
        frames_to_send=[TranscriptionFrame(text="halo, apa kabar and how are you", user_id="u", timestamp="t")],
        expected_down_frames=[TextFrame, TTSAudioRawFrame],
    )
    kinds = [type(f).__name__ for f in received_down]
    print(f"      frames out of pipeline: {kinds}")
    assert any(isinstance(f, TTSAudioRawFrame) for f in received_down), "no TTS audio produced"
    print("      pipeline produced TTS audio end-to-end")
    return True


def main():
    try:
        ok = asyncio.run(_run())
    except Exception as e:
        print(f"\nSMOKE TEST FAILED: {type(e).__name__}: {e}")
        return 1
    print("\nSMOKE TEST PASSED" if ok else "\nSMOKE TEST FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
