#!/usr/bin/env bash
# (NOTE: this is the DigitalOcean bot deploy script — unrelated to the iOS build.)
# Provision a fresh DigitalOcean Droplet (Ubuntu 22.04/24.04) to run the VOID_AI /
# Nova voice bot. Run as root on the Droplet, from a dir containing the pipecat-agent
# source + a .env (SONIOX_API_KEY, OPENROUTER_API_KEY — NO TURN needed on a public IP).
#
#   bash do-setup.sh
#
# Runs ONE CONTAINER PER INSTANCE from instances.conf, each with its persona and mode
# passed explicitly. Earlier versions started a single container and left the second
# persona to a hand-typed `docker run`, so the droplet's actual modes lived nowhere
# anybody could read them — which is how :8080 and :8081 ended up mismatched with docs.
set -euo pipefail

cd "$(dirname "$0")/.."   # repo's pipecat-agent/ — the Dockerfile's build context

# --- Instance table (name port persona mode) --------------------------------------------
INSTANCES_DEFAULT='cascade 8080 nova cascade voice
gemini 8081 kaira gemini voice
vesper 8082 vesper gemini voice'

read_instances() {
    local src="" f
    if [ -n "${INSTANCES:-}" ]; then
        src=$INSTANCES
    else
        for f in "${INSTANCES_FILE:-}" ./deploy/instances.conf ./instances.conf; do
            if [ -n "$f" ] && [ -f "$f" ]; then src=$(cat "$f"); break; fi
        done
    fi
    [ -n "$src" ] || src=$INSTANCES_DEFAULT
    printf '%s\n' "$src" | sed 's/#.*//' | awk 'NF>=4 {print $1, $2, $3, $4, (NF>=5 ? $5 : "app")}'
}

# --- Docker ---
if ! command -v docker >/dev/null 2>&1; then
    curl -fsSL https://get.docker.com | sh
fi

# --- Firewall ---
# SSH + HTTP signaling + WebRTC media. aiortc binds a RANDOM high UDP port per call, so
# open the whole ephemeral range. (This box only runs the bot; that's fine.)
if command -v ufw >/dev/null 2>&1; then
    ufw allow 22/tcp
    while read -r name port persona mode surface; do
        ufw allow "$port/tcp"   # the native app talks plain HTTP straight to these
    done < <(read_instances)
    ufw allow 10000:65535/udp
    ufw --force enable
fi

# --- Build once, run one container per instance ---
# --network host is ESSENTIAL: it puts the container directly on the Droplet's public
# interface so aiortc advertises the PUBLIC IP as its host candidate and can use any UDP
# port. Without it, Docker's bridge NAT re-creates the exact problem we left fly to escape.
docker build -t voidai-bot .

# The instance table is authoritative, so clear EVERY previous voidai-bot* container --
# including ones from an older naming scheme. Under --network host a survivor still holds
# its port: a leftover voidai-bot-kaira on :8081 would make the new gemini container fail
# to bind, and the bare voidai-bot did the same for :8080 before the rename.
for c in $(docker ps -aq --filter 'name=^voidai-bot' 2>/dev/null); do
    docker rm -f "$c" >/dev/null 2>&1 || true
done

while read -r name port persona mode surface; do
    echo "==> $name: persona=$persona mode=$mode surface=$surface port=$port"
    docker rm -f "voidai-bot-$name" >/dev/null 2>&1 || true
    docker run -d --name "voidai-bot-$name" \
        --network host \
        --env-file .env \
        -e "KAIRA_PERSONA=$persona" \
        -e "KAIRA_MODE=$mode" \
        -e "KAIRA_SURFACE=$surface" \
        --restart unless-stopped \
        voidai-bot \
        uv run bot.py --host 0.0.0.0 --port "$port" -t webrtc
done < <(read_instances)

sleep 3
echo "----------------------------------------------------------------"
docker ps --filter name=voidai-bot --format '  {{.Names}}: {{.Status}}'
IP=$(curl -fsS --max-time 10 http://169.254.169.254/metadata/v1/interfaces/public/0/ipv4/address 2>/dev/null \
     || curl -s ifconfig.me || echo '?')
echo "  public IP: $IP"
while read -r name port persona mode surface; do
    echo "  $name ($persona/$mode): http://$IP:$port/api/offer   ·   health: /status"
done < <(read_instances)
echo "  HTTPS for phone browsers: bash deploy/tls-setup.sh"
echo "  logs: docker logs -f voidai-bot-<name>"
echo "----------------------------------------------------------------"
