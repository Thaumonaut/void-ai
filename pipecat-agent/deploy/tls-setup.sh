#!/usr/bin/env bash
# Put HTTPS in front of the bots so the PREBUILT BROWSER CLIENT (/client) works from a phone.
#
# WHY THIS EXISTS
# The runner's prebuilt UI is a browser mic app: it calls navigator.mediaDevices.getUserMedia.
# Browsers only expose mediaDevices on a SECURE CONTEXT (https, or localhost). Served from
# http://<droplet-ip>:8080/client it fails silently — the bundle has no isSecureContext guard,
# so the page just sits there "loading" forever. /status still answers, which makes it look
# like a network problem. It isn't: it's the origin.
#
# WHAT THIS DOES
#   1. Gets a real Let's Encrypt cert WITHOUT owning a domain, via sslip.io wildcard DNS
#      (cascade.<ip>.sslip.io resolves to <ip>, so HTTP-01 just works).
#   2. Reverse-proxies one hostname per instance in instances.conf, named for the axis you
#      are testing — https://cascade.<ip>.sslip.io and https://gemini.<ip>.sslip.io sit side
#      by side, so A/B-ing the two pipelines is two bookmarks.
#   3. Injects the `x-bot-token` header upstream, because the browser client can't send it —
#      so /api/offer stops 403ing without weakening the bot.
#   4. Puts HTTP basic auth in front, since the injected token would otherwise make the bots
#      open to anyone who guesses the hostname.
#
# It is ADDITIVE: it does not touch, rebuild, or restart the running containers, and the
# plain-HTTP ports the native app uses keep working exactly as before.
#
#   bash tls-setup.sh                 # password auto-generated and printed
#   WEB_PASSWORD=hunter2 bash tls-setup.sh
#
set -euo pipefail

ENV_FILE=${ENV_FILE:-/opt/voidai/.env}
CADDYFILE=${CADDYFILE:-/etc/caddy/Caddyfile}
WEB_USER=${WEB_USER:-void}

# --- Instance table (name port persona mode) --------------------------------------------
# Defaults are embedded because this script is normally run through `curl | bash`, where
# there is no sibling instances.conf to read.
INSTANCES_DEFAULT='cascade 8080 nova cascade voice
gemini 8081 kaira gemini voice
vesper 8082 vesper gemini voice'

read_instances() {
    local src="" f
    if [ -n "${INSTANCES:-}" ]; then
        src=$INSTANCES
    else
        for f in "${INSTANCES_FILE:-}" ./instances.conf ./deploy/instances.conf \
                 /opt/voidai/deploy/instances.conf; do
            if [ -n "$f" ] && [ -f "$f" ]; then src=$(cat "$f"); break; fi
        done
    fi
    [ -n "$src" ] || src=$INSTANCES_DEFAULT
    printf '%s\n' "$src" | sed 's/#.*//' | awk 'NF>=4 {print $1, $2, $3, $4, (NF>=5 ? $5 : "app")}'
}

# --- Public IP (sslip.io encodes it in the hostname, so this must be the real one) ---
# Ask the droplet itself before asking the internet: the DO metadata service is link-local,
# always reachable, and authoritative. The external echo services are only a fallback for
# non-DO hosts — an earlier version led with them and failed on a droplet that could not
# reach them, which is exactly the machine that needs no help identifying itself.
_valid_ip() { printf '%s' "${1:-}" | grep -qE '^([0-9]{1,3}\.){3}[0-9]{1,3}$'; }

_detect_ip() {
    local md="http://169.254.169.254/metadata/v1" ip
    # A reserved/floating IP is what users actually reach, so it wins over the interface
    # address. Returns 404 (and so an empty ip) when the droplet has none.
    for path in floating_ip/ipv4/ip_address interfaces/public/0/ipv4/address; do
        ip=$(curl -fsS --max-time 5 "$md/$path" 2>/dev/null | tr -d '[:space:]') || ip=""
        _valid_ip "$ip" && { printf '%s' "$ip"; return 0; }
    done
    for svc in https://ifconfig.me https://api.ipify.org https://icanhazip.com; do
        ip=$(curl -fsS --max-time 10 "$svc" 2>/dev/null | tr -d '[:space:]') || ip=""
        _valid_ip "$ip" && { printf '%s' "$ip"; return 0; }
    done
    # Last resort: whatever source address the default route picks.
    ip=$(ip -4 route get 1.1.1.1 2>/dev/null | sed -n 's/.*src \([0-9.]*\).*/\1/p' | head -1) || ip=""
    _valid_ip "$ip" && { printf '%s' "$ip"; return 0; }
    return 1
}

IP=${DROPLET_IP:-}
if [ -z "$IP" ]; then
    IP=$(_detect_ip) || IP=""
fi
if ! _valid_ip "$IP"; then
    cat >&2 <<'MSG'
Could not determine this host's public IPv4 address.

Re-run with it set explicitly (the address you SSH to):
  curl -fsSL <this-script-url> | DROPLET_IP=203.0.113.10 bash
MSG
    exit 1
fi
echo "==> Public IP: $IP  (override with DROPLET_IP=... if that is not how you reach this box)"

# --- The bots' shared token, so Caddy can present it on the browser's behalf ---
BOT_TOKEN=""
if [ -f "$ENV_FILE" ]; then
    # Last assignment wins (same as dotenv), minus surrounding quotes and any CRLF.
    BOT_TOKEN=$(sed -n 's/^[[:space:]]*BOT_AUTH_TOKEN[[:space:]]*=[[:space:]]*//p' "$ENV_FILE" \
        | tail -1 | tr -d '\r' | sed -e 's/^"//' -e 's/"$//' -e "s/^'//" -e "s/'\$//")
fi
if [ -n "$BOT_TOKEN" ]; then
    echo "==> Found BOT_AUTH_TOKEN in $ENV_FILE — Caddy will inject it upstream"
else
    echo "==> No BOT_AUTH_TOKEN in $ENV_FILE — assuming the bots are unauthenticated"
fi

WEB_PASSWORD=${WEB_PASSWORD:-$(head -c 18 /dev/urandom | base64 | tr -d '/+=' | head -c 20)}

# --- Firewall: ACME HTTP-01 needs :80, the browser needs :443 ---
# Media is unchanged — it stays on the already-open 10000:65535/udp range, direct to aiortc.
if command -v ufw >/dev/null 2>&1; then
    ufw allow 80/tcp
    ufw allow 443/tcp
fi

# --- Caddy (host install, not docker: the bots run --network host on loopback) ---
if ! command -v caddy >/dev/null 2>&1; then
    echo "==> Installing Caddy"
    apt-get update -qq
    apt-get install -y -qq debian-keyring debian-archive-keyring apt-transport-https curl gnupg
    curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
        | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
    curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
        | tee /etc/apt/sources.list.d/caddy-stable.list >/dev/null
    apt-get update -qq
    apt-get install -y -qq caddy
fi

PW_HASH=$(caddy hash-password --plaintext "$WEB_PASSWORD")

# --- Caddyfile: one site per instance ---
# `basic_auth` is the Caddy >=2.8 spelling; older builds want `basicauth`. Emit the modern
# one and fall back only if validation rejects it, so this works on whatever apt ships.
write_caddyfile() {
    local auth_directive=$1
    local token_line=""
    [ -n "$BOT_TOKEN" ] && token_line="        header_up x-bot-token \"$BOT_TOKEN\""

    {
        echo "# Generated by pipecat-agent/deploy/tls-setup.sh — regenerate rather than hand-editing."
        echo
        while read -r name port persona mode surface; do
            cat <<EOF
# $name = $persona / $mode
$name.$IP.sslip.io {
    $auth_directive {
        $WEB_USER $PW_HASH
    }
    reverse_proxy 127.0.0.1:$port {
$token_line
    }
}

EOF
        done < <(read_instances)
    } >"$CADDYFILE"
}

write_caddyfile "basic_auth"
if ! caddy validate --config "$CADDYFILE" >/dev/null 2>&1; then
    echo "==> 'basic_auth' rejected; falling back to the pre-2.8 'basicauth' spelling"
    write_caddyfile "basicauth"
    caddy validate --config "$CADDYFILE"
fi

systemctl enable --now caddy
systemctl reload caddy || systemctl restart caddy

# --- Verify (ACME can take a few seconds on the first request for each host) ---
echo "==> Waiting for certificates"
while read -r name port persona mode surface; do
    host="$name.$IP.sslip.io"
    ok=""
    for _ in $(seq 1 20); do
        code=$(curl -fsS -o /dev/null -w '%{http_code}' --max-time 10 \
            -u "$WEB_USER:$WEB_PASSWORD" "https://$host/status" 2>/dev/null) || code=""
        if [ "$code" = "200" ]; then ok=1; break; fi
        sleep 3
    done
    if [ -n "$ok" ]; then
        echo "    OK   https://$host/status  ($persona/$mode)"
    else
        echo "    WARN https://$host/status not ready — is a bot listening on :$port?"
    fi
done < <(read_instances)

echo "----------------------------------------------------------------"
echo "Open these on your phone (any network, cellular included):"
echo
while read -r name port persona mode surface; do
    printf '  %-8s %-12s https://%s.%s.sslip.io/client/\n' "$name" "($persona/$mode)" "$name" "$IP"
done < <(read_instances)
cat <<EOF

  username: $WEB_USER
  password: $WEB_PASSWORD

Save the password — it is not stored anywhere in plaintext. To change it:
  WEB_PASSWORD=<new> bash tls-setup.sh

The native app is unaffected: it still talks plain HTTP to the same ports.
If a page loads and the mic works but the bot never connects, media (UDP) is being
blocked on that network — set TURN_TOKEN_ID / TURN_API_TOKEN in $ENV_FILE and restart
the containers to get a relay path.
  logs: docker logs -f voidai-bot-<name>   ·   caddy: journalctl -u caddy -f
----------------------------------------------------------------
EOF
