# VOID_AI bot — DigitalOcean deploy

A public-IP host is what self-hosted WebRTC media wants: aiortc advertises the Droplet's
public IP directly, so there's **no TURN / coturn / relay** at all. Works for any client NAT.

## 1. Create the Droplet (you)
- **Ubuntu 24.04**, **2 GB RAM** (the bot pulls torch + ONNX models; 1 GB is tight).
- Region near your users (e.g. `sfo`/`sgp` for the Indonesia rollout).
- **SSH keys:** add the deploy key below (so the deploy can be driven for you):

  ```
  ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJlKW9gWOUflYabFLlv009IXd+IVgJTCQ+kdVayt/sY9 voidai-do-deploy
  ```
  (Add your own key too if you want direct access.)
- Note the Droplet's **public IPv4**.

## 2. Deploy (driven from your Mac over SSH)
```bash
# from Rust-Mobile/../valdi/pipecat-agent
DROPLET=<public-ip>
rsync -az --delete \
  --exclude .venv --exclude .git --exclude __pycache__ \
  -e "ssh -i ~/.ssh/voidai_do -o StrictHostKeyChecking=accept-new" \
  ./ root@$DROPLET:/opt/voidai/
ssh -i ~/.ssh/voidai_do root@$DROPLET "cd /opt/voidai && bash deploy/do-setup.sh"
```
`.env` (SONIOX_API_KEY, OPENROUTER_API_KEY — **no TURN needed**) rsyncs along with the code.

## 3. Verify
```bash
curl http://$DROPLET:8080/status            # {"status":"ready",...}
ssh -i ~/.ssh/voidai_do root@$DROPLET "docker logs -f voidai-bot"
```

## 4. Point the app at it
`kaira-slint/src/lib.rs` → iOS `DEFAULT_BOT = "http://<DROPLET-IP>:8080/api/offer"`, rebuild
in Xcode. (ATS already allows plain HTTP; the client uses no TURN/`/ice`.)

## 5. Browser access from a phone (`/client` over HTTPS)
The runner's prebuilt UI is a **browser mic app** — it calls `navigator.mediaDevices.getUserMedia`,
which browsers only expose on a **secure context** (HTTPS, or `localhost`). So `/client` works at
`http://localhost:7860` but hangs forever at `http://<ip>:8080/client` — the bundle has no
`isSecureContext` guard, so it never reports why. `/status` still answers, which makes it look like
a network fault. It isn't. (The **native app** is a browser-free client, so none of this affects it.)

`tls-setup.sh` fixes that without owning a domain, using `sslip.io` wildcard DNS
(`nova.<ip>.sslip.io` resolves to `<ip>`, so Let's Encrypt HTTP-01 just works):

```bash
ssh root@$DROPLET "cd /opt/voidai && bash deploy/tls-setup.sh"   # prints the password
```

It is **additive** — it installs Caddy alongside the running containers and never rebuilds or
restarts them; the plain-HTTP `:8080`/`:8081` endpoints the app uses are untouched.

One hostname per instance, named for the **axis you are testing** rather than the persona,
so the two pipelines sit side by side as two bookmarks:

```
https://cascade.<ip>.sslip.io/client/   ->  127.0.0.1:8080   (nova   / cascade)
https://gemini.<ip>.sslip.io/client/    ->  127.0.0.1:8081   (kaira  / gemini)
https://vesper.<ip>.sslip.io/client/    ->  127.0.0.1:8082   (vesper / gemini)
```

Two things it handles that HTTPS alone wouldn't:
- **`x-bot-token`** — the prebuilt client can't send it, so `/api/offer` would 403 under
  `BOT_AUTH_TOKEN`. Caddy reads the token from `.env` and injects it upstream.
- **HTTP basic auth** — since that injection would otherwise leave the bots open to anyone who
  guesses the hostname. Set your own with `WEB_PASSWORD=… bash deploy/tls-setup.sh`.

## Instances

`instances.conf` is the single source of truth for both scripts — `do-setup.sh` reads it to
decide what to run, `tls-setup.sh` reads it to decide what to expose:

```
# name     port  persona  mode     surface
cascade    8080  nova     cascade  voice
gemini     8081  kaira    gemini   voice
vesper     8082  vesper   gemini   voice
```

`surface` picks how the agent delivers a tool result, via `KAIRA_SURFACE`:

- **`voice`** — there is no screen. The agent SAYS the answer: the best two or three, with the
  detail that decides it. Use this for the pipecat browser client and any prompt/voice test.
- **`app`** (the default when the column is absent) — the Slint client, which really renders the
  map/images/products, so the agent reacts to the display instead of reciting it.

Only wording changes — same persona, same voice, same tools — so a voice-mode session is still
a faithful test of the prompt. This matters because the UI messages go out over RTVI either
way: with no client rendering them, screen-shaped wording like *"the third one's highway
robbery"* points at an empty room and the user never hears the actual result.

Each row becomes a container (`voidai-bot-<name>`) started with `KAIRA_PERSONA` and
`KAIRA_MODE` passed explicitly, and a matching HTTPS hostname. Add a row, re-run both
scripts, and the port, firewall rule, container and certificate all follow.

This exists because the modes used to live only in hand-typed `docker run` lines, so nothing
on disk recorded which pipeline a port was actually serving — and the droplet quietly drifted
from what this README claimed. `tls-setup.sh` embeds the same table as a fallback, since it is
normally run through `curl | bash` with no sibling file to read.

## Notes
- `--network host` in the run command is essential — it keeps the container on the public
  interface so aiortc sees the real IP (Docker's bridge NAT would re-break it).
- HTTP on a bare IP is fine for a native app; move to a domain + Caddy auto-TLS for HTTPS later.
- fly deploy stays as-is (cone-NAT fallback); this Droplet becomes the primary.
