# syntax=docker/dockerfile:1
# Pipecat SmallWebRTC voice bot for fly.io. Media rides Cloudflare TURN (see bot.py),
# so this is just a normal HTTPS app — no UDP exposure needed.
FROM ghcr.io/astral-sh/uv:python3.12-bookworm-slim

ENV PYTHONUNBUFFERED=1 \
    UV_COMPILE_BYTECODE=1 \
    UV_LINK_MODE=copy

# libgomp1 = the OpenMP runtime onnxruntime needs (Silero VAD + smart-turn ONNX).
RUN apt-get update && apt-get install -y --no-install-recommends libgomp1 \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Deps first for layer caching. --frozen honors uv.lock exactly.
COPY pyproject.toml uv.lock ./
RUN uv sync --frozen --no-install-project --no-dev

# App code. The Silero VAD (2.2M) + smart-turn v3 (8.3M) ONNX models are bundled inside
# the pipecat wheel — no runtime download, so cold start does no model fetch.
COPY . .
RUN uv sync --frozen --no-dev

EXPOSE 8080
# The runner binds uvicorn to args.host/args.port (defaults localhost:7860) — override to
# 0.0.0.0:8080 for fly. -t webrtc = WebRTC transport only.
CMD ["uv", "run", "bot.py", "--host", "0.0.0.0", "--port", "8080", "-t", "webrtc"]
