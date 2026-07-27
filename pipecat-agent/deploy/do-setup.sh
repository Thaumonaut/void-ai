#!/usr/bin/env bash
# (NOTE: this is the DigitalOcean bot deploy script — unrelated to the iOS build.)
# Provision a fresh DigitalOcean Droplet (Ubuntu 22.04/24.04) to run the VOID_AI /
# Nova voice bot. Run as root on the Droplet, from a dir containing the pipecat-agent
# source + a .env (SONIOX_API_KEY, OPENROUTER_API_KEY — NO TURN needed on a public IP).
#
#   bash do-setup.sh
set -euxo pipefail

# --- Docker ---
if ! command -v docker >/dev/null 2>&1; then
    curl -fsSL https://get.docker.com | sh
fi

# --- Firewall ---
# SSH + HTTP signaling + WebRTC media. aiortc binds a RANDOM high UDP port per call, so
# open the whole ephemeral range. (This box only runs the bot; that's fine.)
if command -v ufw >/dev/null 2>&1; then
    ufw allow 22/tcp
    ufw allow 8080/tcp
    ufw allow 10000:65535/udp
    ufw --force enable
fi

# --- Build + run ---
# --network host is ESSENTIAL: it puts the container directly on the Droplet's public
# interface so aiortc advertises the PUBLIC IP as its host candidate and can use any UDP
# port. Without it, Docker's bridge NAT re-creates the exact problem we left fly to escape.
docker build -t voidai-bot .
docker rm -f voidai-bot >/dev/null 2>&1 || true
docker run -d --name voidai-bot \
    --network host \
    --env-file .env \
    --restart unless-stopped \
    voidai-bot

sleep 3
echo "----------------------------------------------------------------"
docker ps --filter name=voidai-bot --format '  {{.Names}}: {{.Status}}'
echo "  public IP: $(curl -s ifconfig.me || echo '?')"
echo "  signaling: http://<that-ip>:8080/api/offer   ·   health: /status"
echo "  logs: docker logs -f voidai-bot"
echo "----------------------------------------------------------------"
