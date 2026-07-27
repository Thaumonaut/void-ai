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

## Notes
- `--network host` in the run command is essential — it keeps the container on the public
  interface so aiortc sees the real IP (Docker's bridge NAT would re-break it).
- HTTP on a bare IP is fine for a native app; move to a domain + Caddy auto-TLS for HTTPS later.
- fly deploy stays as-is (cone-NAT fallback); this Droplet becomes the primary.
