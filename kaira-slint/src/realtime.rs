//! Realtime voice-agent client (Pipecat SmallWebRTC over WebRTC). Android-only —
//! webrtc-rs is an android-only dep, cross-compile + on-device ICE/DTLS handshake
//! to the bot both spike-verified on aarch64 API 24.
//!
//! Session: PeerConnection → add a mic Opus track (cpal → resample 48k → 20ms Opus
//! frames → write_sample) → inbound track: read RTP → Opus-decode → play (rodio).
//! Non-trickle signaling: gather ICE, POST the offer to `/api/offer`, apply answer.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_OPUS};
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

use crate::opus::{Decoder, Encoder, SAMPLE_RATE};
use crate::tts::Resampler;

/// Status/state callback — must be Send+Sync so it can ride into the webrtc handlers.
pub type StatusCb = Arc<dyn Fn(String) + Send + Sync>;
/// Mic input level (0..1 peak), pushed ~15×/s so the UI can draw a live meter.
pub type LevelCb = Arc<dyn Fn(f32) + Send + Sync>;
/// Full rendered conversation transcript, pushed whenever a new line lands.
pub type TranscriptCb = Arc<dyn Fn(String) + Send + Sync>;
/// UI-control message from Nova (the bot). Carries the `data` object of an RTVI
/// `server-message` as a JSON string, e.g. `{"op":"map","query":"…","pins":[…]}`.
/// The UI layer parses `op` and drives the view surface. See UI_CONTRACT.md.
pub type UiControlCb = Arc<dyn Fn(String) + Send + Sync>;

/// Accumulates the conversation into speaker-labelled lines. Consecutive text from
/// the SAME speaker coalesces into one line (Soniox emits user finals in chunks and
/// the bot in per-sentence pieces); a speaker change commits the open line.
#[derive(Default)]
struct Transcript {
    committed: Vec<String>,
    cur_user: Option<bool>, // Some(true)=you, Some(false)=SassBot
    cur_text: String,
}

impl Transcript {
    fn add(&mut self, is_user: bool, piece: &str) {
        let piece = piece.trim();
        if piece.is_empty() {
            return;
        }
        if self.cur_user != Some(is_user) {
            self.commit();
            self.cur_user = Some(is_user);
        }
        if !self.cur_text.is_empty() && !self.cur_text.ends_with(' ') {
            self.cur_text.push(' ');
        }
        self.cur_text.push_str(piece);
    }
    fn commit(&mut self) {
        if let Some(u) = self.cur_user {
            if !self.cur_text.trim().is_empty() {
                let who = if u { "🗣  You" } else { "🤖  Nova" };
                self.committed.push(format!("{who}\n{}", self.cur_text.trim()));
            }
        }
        self.cur_text.clear();
        self.cur_user = None;
    }
    /// Committed lines + the currently-open line, capped to the last ~40 turns.
    fn render(&self) -> String {
        let mut lines = self.committed.clone();
        if let Some(u) = self.cur_user {
            if !self.cur_text.trim().is_empty() {
                let who = if u { "🗣  You" } else { "🤖  Nova" };
                lines.push(format!("{who}\n{}", self.cur_text.trim()));
            }
        }
        let n = lines.len();
        if n > 40 {
            lines = lines[n - 40..].to_vec();
        }
        lines.join("\n\n")
    }
}

pub enum Cmd {
    Connect(String),
    Disconnect,
}

// User mute: when true the pump sends silence instead of the mic (single active
// session, so a module-level flag is simpler than threading an Arc through the pump).
static MUTED: AtomicBool = AtomicBool::new(false);
// True while Nova's audio is actually still coming out of the speaker (sink not empty).
// The mic pump gates on this so the speaker's playout tail can't echo back into the mic
// and make her interrupt herself — precise, not a fixed guess.
static AGENT_PLAYING: AtomicBool = AtomicBool::new(false);
// Tap-the-waveform to cut Nova off. Set by the UI; the dc ping loop picks it up and
// sends {"op":"interrupt"} so the bot flushes its TTS/LLM. Because the mic is muted
// while Nova plays (AGENT_PLAYING), this tap is the ONLY way to interrupt her — voice
// barge-in is intentionally off (it's what made her hear her own echo).
static INTERRUPT_REQ: AtomicBool = AtomicBool::new(false);
// Same tap flushes the LOCAL playback buffer so she goes silent instantly, instead of
// draining the already-decoded audio while the bot's interrupt round-trips.
static FLUSH_PLAYBACK: AtomicBool = AtomicBool::new(false);

pub struct Realtime {
    tx: std::sync::mpsc::Sender<Cmd>,
}

impl Realtime {
    pub fn new(
        status: StatusCb,
        level: LevelCb,
        agent_level: LevelCb,
        transcript: TranscriptCb,
        ui_control: UiControlCb,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
        std::thread::spawn(move || rt_thread(rx, status, level, agent_level, transcript, ui_control));
        Self { tx }
    }
    pub fn connect(&self, url: String) {
        let _ = self.tx.send(Cmd::Connect(url));
    }
    pub fn disconnect(&self) {
        let _ = self.tx.send(Cmd::Disconnect);
    }
    /// Mute/unmute the outbound mic (the bot hears silence while muted).
    pub fn set_muted(&self, muted: bool) {
        MUTED.store(muted, Ordering::SeqCst);
    }
    /// User tapped the waveform to shut Nova up: flush the local playout NOW (instant
    /// silence) and ask the bot to stop generating (over the data channel).
    pub fn interrupt(&self) {
        FLUSH_PLAYBACK.store(true, Ordering::SeqCst);
        INTERRUPT_REQ.store(true, Ordering::SeqCst);
    }
}

fn rt_thread(
    rx: std::sync::mpsc::Receiver<Cmd>,
    status: StatusCb,
    level: LevelCb,
    agent_level: LevelCb,
    transcript: TranscriptCb,
    ui_control: UiControlCb,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            status(format!("realtime: runtime init failed: {e}"));
            return;
        }
    };
    // Track the live session so we can FULLY tear it down (pc.close + threads)
    // before starting a new one — otherwise a reconnect races the old still-open
    // connection and the server rejects it ("existing connection active").
    let mut current: Option<(Arc<AtomicBool>, tokio::task::JoinHandle<()>)> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Connect(url) => {
                if let Some((s, h)) = current.take() {
                    s.store(true, Ordering::SeqCst);
                    let _ = runtime.block_on(h); // wait for clean close before reconnecting
                }
                let stop = Arc::new(AtomicBool::new(false));
                let st = status.clone();
                let lv = level.clone();
                let al = agent_level.clone();
                let tr = transcript.clone();
                let uc = ui_control.clone();
                let s = stop.clone();
                let h = runtime.spawn(async move { run_session(url, st, lv, al, tr, uc, s).await });
                current = Some((stop, h));
            }
            Cmd::Disconnect => {
                if let Some((s, h)) = current.take() {
                    s.store(true, Ordering::SeqCst);
                    let _ = runtime.block_on(h); // wait for the PC + audio to fully release
                }
                status("realtime: disconnected".into());
            }
        }
    }
}

async fn run_session(
    url: String,
    status: StatusCb,
    level: LevelCb,
    agent_level: LevelCb,
    transcript: TranscriptCb,
    ui_control: UiControlCb,
    stop: Arc<AtomicBool>,
) {
    status("realtime: connecting…".into());
    transcript(String::new()); // clear last session's transcript

    let mut m = MediaEngine::default();
    if m.register_default_codecs().is_err() {
        status("realtime: codec init failed".into());
        return;
    }
    let mut registry = Registry::new();
    registry = match register_default_interceptors(registry, &mut m) {
        Ok(r) => r,
        Err(_) => {
            status("realtime: interceptor init failed".into());
            return;
        }
    };
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build();

    // STUN only. The BOT advertises a Cloudflare TURN relay candidate (with the
    // nonce-rotation fix), so it can channel-bind our public (srflx) candidate and
    // receive our media directly — no need for the client to also allocate a relay
    // (which stalled ICE gathering while it tried all of Cloudflare's TURN URLs).
    // If a symmetric-NAT/CGNAT case later needs a client relay, re-add it via /ice but
    // pinned to the single UDP turn: URL and with a short gather timeout.
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let pc = match api.new_peer_connection(config).await {
        Ok(pc) => Arc::new(pc),
        Err(e) => {
            status(format!("realtime: peer connection failed: {e}"));
            return;
        }
    };

    // --- RTVI data channel ('chat') for live transcripts + speaking state ---
    // Pipecat's PipelineTask auto-runs an RTVIProcessor that emits transcription +
    // speaking events over a data channel the CLIENT must open (label 'chat',
    // ordered) BEFORE the offer so it lands in the SDP. We must also send a "ping"
    // at least every 3s or the bot marks us disconnected and stops sending events
    // (and its message queue backs up). The playground pings every 1s.
    let dc = match pc
        .create_data_channel(
            "chat",
            Some(RTCDataChannelInit {
                ordered: Some(true),
                ..Default::default()
            }),
        )
        .await
    {
        Ok(d) => d,
        Err(e) => {
            status(format!("realtime: data channel failed: {e}"));
            return;
        }
    };
    {
        // Keepalive ping loop starts once the channel opens.
        let ping_dc = dc.clone();
        let ping_stop = stop.clone();
        dc.on_open(Box::new(move || {
            Box::pin(async move {
                eprintln!("[dc] 'chat' channel OPEN — starting ping keepalive");
                // Send the phone's location so the bot's directions/place lookups start
                // from where the user actually is (not a hardcoded default). Android reads
                // LocationManager; iOS reads CLLocationManager — both best-effort (None on a
                // cold cache, which just leaves the bot on its Seattle fallback).
                #[cfg(target_os = "android")]
                let here = crate::native_stt::last_location();
                #[cfg(target_os = "ios")]
                let here = crate::ios_location::last_location();
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                let here: Option<(f64, f64)> = None;
                if let Some((lat, lng)) = here {
                    // "type" key required by SmallWebRTC's on_message (see interrupt below).
                    let msg = format!("{{\"type\":\"location\",\"lat\":{lat},\"lng\":{lng}}}");
                    let _ = ping_dc.send_text(msg).await;
                    eprintln!("[dc] sent location {lat},{lng}");
                }
                // Poll every 80ms so a waveform tap → interrupt feels instant, but only
                // emit the keepalive "ping" ~once a second (the bot just needs <3s cadence).
                let mut n: u64 = 0;
                let mut ticks: u32 = 0;
                while !ping_stop.load(Ordering::SeqCst) {
                    if INTERRUPT_REQ.swap(false, Ordering::SeqCst) {
                        // Pipecat's SmallWebRTC on_message REQUIRES a "type" key (it does
                        // json["type"] unconditionally — a bare {"op":..} raises KeyError
                        // and the message is dropped before on_app_message ever sees it).
                        let _ = ping_dc.send_text("{\"type\":\"interrupt\"}".to_string()).await;
                        eprintln!("[dc] sent interrupt");
                    }
                    if ticks % 12 == 0 {
                        if ping_dc.send_text(format!("ping: {n}")).await.is_err() {
                            break;
                        }
                        n += 1;
                    }
                    ticks += 1;
                    tokio::time::sleep(Duration::from_millis(80)).await;
                }
            })
        }));
    }
    {
        // Parse inbound RTVI messages → conversation transcript + UI-control.
        let ts: Arc<Mutex<Transcript>> = Arc::new(Mutex::new(Transcript::default()));
        let tr = transcript.clone();
        let uc = ui_control.clone();
        dc.on_message(Box::new(move |msg: DataChannelMessage| {
            let ts = ts.clone();
            let tr = tr.clone();
            let uc = uc.clone();
            Box::pin(async move {
                let v: serde_json::Value = match serde_json::from_slice(&msg.data) {
                    Ok(v) => v,
                    Err(_) => return, // ignore non-JSON (our own pings never echo)
                };
                let label = v.get("label").and_then(|l| l.as_str()).unwrap_or("");
                if label != "rtvi-ai" {
                    return;
                }
                let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let data = v.get("data");
                let text = |k: &str| {
                    data.and_then(|d| d.get(k))
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                let changed = match ty {
                    "user-transcription" => {
                        let is_final = data
                            .and_then(|d| d.get("final"))
                            .and_then(|f| f.as_bool())
                            .unwrap_or(false);
                        let t = text("text");
                        if is_final && !t.is_empty() {
                            ts.lock().unwrap().add(true, &t);
                            true
                        } else {
                            false
                        }
                    }
                    // Per-sentence assembled bot speech — the cleanest bot line source.
                    "bot-transcription" => {
                        let t = text("text");
                        if !t.is_empty() {
                            ts.lock().unwrap().add(false, &t);
                            true
                        } else {
                            false
                        }
                    }
                    // Nova driving the view surface — hand the `data` object (with `op`)
                    // to the UI layer, which parses it and updates the views. See UI_CONTRACT.md.
                    "server-message" => {
                        if let Some(d) = data {
                            uc(d.to_string());
                        }
                        false
                    }
                    _ => false,
                };
                if changed {
                    let rendered = ts.lock().unwrap().render();
                    tr(rendered);
                }
            })
        }));
    }

    // --- outbound mic track (Opus 48 kHz) ---
    let out_track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: 48000,
            channels: 2, // WebRTC declares Opus stereo in SDP; we send mono packets.
            ..Default::default()
        },
        "audio".to_owned(),
        "kaira-mic".to_owned(),
    ));
    if pc
        .add_track(out_track.clone() as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .is_err()
    {
        status("realtime: add_track failed".into());
        return;
    }

    // --- inbound audio: read RTP → Opus decode → play ---
    let play_q: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
    // Last time Kaira's audio arrived — the mic is gated (half-duplex) while she's
    // speaking so the mic can't capture + echo her voice back and interrupt her.
    let last_inbound: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    {
        let dec_q = play_q.clone();
        let li = last_inbound.clone();
        let al = agent_level.clone();
        pc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let dec_q = dec_q.clone();
            let li = li.clone();
            let al = al.clone();
            Box::pin(async move {
                let mut dec = match Decoder::new() {
                    Some(d) => d,
                    None => return,
                };
                loop {
                    match track.read_rtp().await {
                        Ok((pkt, _)) if !pkt.payload.is_empty() => {
                            let mut pcm = Vec::new();
                            if dec.decode(&pkt.payload, &mut pcm) > 0 {
                                // Only mark "bot speaking" (→ half-duplex mic gate) when
                                // the inbound audio is actually LOUD. WebRTC keeps the
                                // track hot with near-silent comfort frames between
                                // utterances; gating on mere presence muted the mic 100%
                                // of the time (sent=0). Energy-gate so the mic opens the
                                // instant she stops talking.
                                let peak = pcm.iter().fold(0f32, |a, &x| a.max(x.abs()));
                                al(peak); // Nova's live level → the UI visualizer
                                dec_q.lock().unwrap().extend(pcm);
                                if peak > 0.05 {
                                    *li.lock().unwrap() = Some(Instant::now());
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            })
        }));
    }

    // --- connection state → status ---
    {
        let st = status.clone();
        let stop_state = stop.clone();
        pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
            st(match s {
                RTCPeerConnectionState::Connected => "connected · talk to Nova".to_string(),
                RTCPeerConnectionState::Connecting => "realtime: connecting…".to_string(),
                RTCPeerConnectionState::Disconnected => "realtime: disconnected".to_string(),
                RTCPeerConnectionState::Failed => "realtime: connection failed".to_string(),
                RTCPeerConnectionState::Closed => "realtime: closed".to_string(),
                _ => "realtime: …".to_string(),
            });
            if matches!(s, RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed) {
                stop_state.store(true, Ordering::SeqCst);
            }
            Box::pin(async {})
        }));
    }

    // --- offer → POST /api/offer → answer ---
    let offer = match pc.create_offer(None).await {
        Ok(o) => o,
        Err(e) => {
            status(format!("realtime: offer failed: {e}"));
            return;
        }
    };
    if pc.set_local_description(offer).await.is_err() {
        status("realtime: set local desc failed".into());
        return;
    }
    let mut gather = pc.gathering_complete_promise().await;
    let _ = tokio::time::timeout(Duration::from_secs(8), gather.recv()).await;
    let local = match pc.local_description().await {
        Some(d) => d,
        None => {
            status("realtime: no local description".into());
            return;
        }
    };
    eprintln!(
        "[dc] offer has data m-line: {}",
        local.sdp.contains("m=application")
    );
    let resp = match reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "sdp": local.sdp, "type": "offer" }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            status(format!("realtime: signaling failed: {e}"));
            return;
        }
    };
    let answer: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            status(format!("realtime: bad answer: {e}"));
            return;
        }
    };
    let ans_sdp = answer["sdp"].as_str().unwrap_or_default().to_string();
    eprintln!(
        "[dc] answer has data m-line: {}",
        ans_sdp.contains("m=application")
    );
    let desc = match RTCSessionDescription::answer(ans_sdp) {
        Ok(d) => d,
        Err(e) => {
            status(format!("realtime: bad answer sdp: {e}"));
            return;
        }
    };
    if pc.set_remote_description(desc).await.is_err() {
        status("realtime: set remote desc failed".into());
        return;
    }

    // --- start audio (playback drain + mic capture) ---
    // Peak mic level since the last UI push (raw capture — BEFORE the half-duplex gate,
    // so the meter shows the mic is alive even while Kaira is speaking).
    let mic_buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let mic_level: Arc<Mutex<f32>> = Arc::new(Mutex::new(0.0));

    // iOS: one Voice-Processing I/O unit does mic + speaker + hardware AEC/NS (cpal's
    // RemoteIO can't, so a fan swamps the VAD and the bot echoes into the mic).
    #[cfg(target_os = "ios")]
    let vpio = crate::ios_vpio::start(play_q.clone(), mic_buf.clone(), mic_level.clone());
    // Everything else: cpal mic capture + rodio playback on their own threads.
    #[cfg(not(target_os = "ios"))]
    let play_handle = {
        let q = play_q.clone();
        let s = stop.clone();
        std::thread::spawn(move || play_thread(q, s))
    };
    #[cfg(not(target_os = "ios"))]
    let mic_handle = {
        let mb = mic_buf.clone();
        let ms = stop.clone();
        let ml = mic_level.clone();
        std::thread::spawn(move || mic_capture(mb, ms, ml))
    };
    // Push the mic level to the UI ~15×/s (read-and-reset so it decays when you stop).
    {
        let ml = mic_level.clone();
        let lv = level.clone();
        let s = stop.clone();
        tokio::spawn(async move {
            while !s.load(Ordering::SeqCst) {
                let peak = {
                    let mut g = ml.lock().unwrap();
                    let v = *g;
                    *g = 0.0;
                    v
                };
                lv(peak);
                tokio::time::sleep(Duration::from_millis(66)).await;
            }
            lv(0.0); // clear the meter on teardown
        });
    }

    // --- pump 20ms Opus frames to the track ON THIS RUNTIME ---
    let mut enc = match Encoder::new(24_000) {
        Some(e) => e,
        None => {
            status("realtime: opus encoder failed".into());
            return;
        }
    };
    const FRAME: usize = 960; // 20 ms @ 48 kHz
    let (mut sent, mut gated, mut enc_fail, mut write_fail) = (0u64, 0u64, 0u64, 0u64);
    let mut last_beat = Instant::now();
    // Last instant Nova was audibly active; the mic gate holds closed for a hangover past it.
    let mut last_bot_active = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        let frame: Option<Vec<f32>> = {
            let mut b = mic_buf.lock().unwrap();
            if b.len() >= FRAME {
                Some(b.drain(..FRAME).collect())
            } else {
                None
            }
        };
        match frame {
            Some(pcm) => {
                // Half-duplex gate: while Kaira's audio arrived in the last ~400 ms she's
                // (still) speaking — send silence instead of the mic so her voice can't
                // echo back and self-interrupt. On iOS the VPIO's hardware AEC removes most
                // of the echo, but enough leaks to trip the bot's turn-detector, so keep
                // the gate here too (trades barge-in for no self-interruption).
                // Gate the mic while Nova's audio is ACTUALLY still playing out of the speaker
                // (AGENT_PLAYING = sink not empty), plus a short acoustic tail after it drains.
                // Precise, so without hardware AEC her voice can't leak into the mic and make
                // her interrupt herself — and the mic re-opens the moment she truly finishes.
                let bot_now = AGENT_PLAYING.load(Ordering::Relaxed)
                    || last_inbound
                        .lock()
                        .unwrap()
                        .map(|t| t.elapsed() < Duration::from_millis(300))
                        .unwrap_or(false);
                if bot_now {
                    last_bot_active = Instant::now();
                }
                // Hangover: hold the mic closed a beat AFTER she truly stops, so the acoustic
                // tail of her last syllable (speaker latency + room reverb, with no hardware AEC
                // on Android) dies before the mic re-opens. Without it that tail leaks back in and
                // the bot transcribes it as a user turn — "she's hearing herself." 650 ms is long
                // enough to swallow the tail; barge-in is via the waveform tap, not the voice.
                let bot_speaking = last_bot_active.elapsed() < Duration::from_millis(650);
                // Send SILENCE (not nothing) when the bot is speaking OR the user muted:
                // keeps the bot's inbound RTP track alive (else it times out + clears) while
                // (a) preventing her voice echoing back and self-interrupting, and (b)
                // honoring an explicit user mute.
                let outbound = if bot_speaking || MUTED.load(Ordering::SeqCst) {
                    gated += 1;
                    std::borrow::Cow::Owned(vec![0f32; FRAME])
                } else {
                    std::borrow::Cow::Borrowed(&pcm)
                };
                if let Some(op) = enc.encode(&outbound) {
                    let sample = Sample {
                        data: op.into(),
                        duration: Duration::from_millis(20),
                        ..Default::default()
                    };
                    match out_track.write_sample(&sample).await {
                        Ok(_) if !bot_speaking => sent += 1,
                        Ok(_) => {}
                        Err(_) => write_fail += 1,
                    }
                } else if !bot_speaking {
                    enc_fail += 1;
                }
            }
            None => tokio::time::sleep(Duration::from_millis(5)).await,
        }
        if last_beat.elapsed() >= Duration::from_secs(1) {
            eprintln!("[tx] sent={sent} gated={gated} enc_fail={enc_fail} write_fail={write_fail} (per ~1s)");
            sent = 0;
            gated = 0;
            enc_fail = 0;
            write_fail = 0;
            last_beat = Instant::now();
        }
    }
    // Full teardown so the next Connect starts clean: close the PC (server discards its
    // side promptly) and release the audio devices.
    let _ = pc.close().await;
    #[cfg(target_os = "ios")]
    if let Some(v) = vpio {
        v.stop();
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = play_handle.join();
        let _ = mic_handle.join();
    }
    status("realtime: closed".into());
}

/// rodio playback of the decoded 48 kHz PCM queue (own thread — OutputStream is !Send).
fn play_thread(queue: Arc<Mutex<VecDeque<f32>>>, stop: Arc<AtomicBool>) {
    let (_stream, handle) = match rodio::OutputStream::try_default() {
        Ok(v) => v,
        Err(_) => return,
    };
    let sink = match rodio::Sink::try_new(&handle) {
        Ok(s) => s,
        Err(_) => return,
    };
    // Nova's TTS comes in quiet. Amplify, but soft-clip (tanh) instead of hard gain so
    // loud peaks saturate smoothly rather than clipping/distorting. ~3.4x on quiet speech,
    // gracefully limited toward ±1 on peaks.
    const GAIN: f32 = 3.0;
    sink.set_volume(1.0);
    // Last time REAL (non-silent) audio played. WebRTC keeps the track hot with near-silent
    // comfort frames between Nova's utterances; those keep the sink non-empty, so gating the
    // mic on `!sink.empty()` alone wedged the mic shut forever → the bot heard only silence
    // and never transcribed anything. Only treat her as "speaking" when loud audio played
    // recently AND the sink is still busy — so the mic re-opens the instant she truly stops.
    let mut last_loud = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        // Waveform tap → drop everything already decoded/queued AND clear the sink so
        // Nova falls silent immediately, not after the buffered tail finishes playing.
        if FLUSH_PLAYBACK.swap(false, Ordering::SeqCst) {
            queue.lock().unwrap().clear();
            sink.clear();
            sink.play(); // clear() pauses the sink — re-arm it for the next turn
            AGENT_PLAYING.store(false, Ordering::Relaxed);
        }
        let mut chunk: Vec<f32> = {
            let mut q = queue.lock().unwrap();
            let n = q.len();
            if n >= 480 {
                q.drain(..n).collect()
            } else {
                Vec::new()
            }
        };
        if !chunk.is_empty() {
            // Peak BEFORE gain, so the threshold matches the on_track inbound gate. Comfort
            // noise sits far below this; her actual voice sits above it.
            let peak = chunk.iter().fold(0f32, |a, &x| a.max(x.abs()));
            if peak > 0.05 {
                last_loud = Instant::now();
            }
            for s in chunk.iter_mut() {
                *s = (*s * GAIN).tanh();
            }
            sink.append(rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, chunk));
        }
        // Speaking = real audio still in flight (loud within the last 500 ms AND not drained).
        let playing = !sink.empty() && last_loud.elapsed() < Duration::from_millis(500);
        AGENT_PLAYING.store(playing, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Capture the mic (cpal), downmix to mono, resample to 48 kHz, append to `buf`, and
/// report the per-callback peak into `mic_level` for the UI meter. Verbose `[mic]`
/// logging so `adb logcat` shows exactly where the input path breaks.
fn mic_capture(buf: Arc<Mutex<Vec<f32>>>, stop: Arc<AtomicBool>, mic_level: Arc<Mutex<f32>>) {
    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            eprintln!("[mic] NO default input device — nothing to capture");
            return;
        }
    };
    let name = device.name().unwrap_or_else(|_| "<unknown>".into());
    let supported = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[mic] default_input_config failed on '{name}': {e}");
            return;
        }
    };
    let in_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let fmt = supported.sample_format();
    eprintln!("[mic] device='{name}' rate={in_rate} ch={channels} fmt={fmt:?} → resample to {SAMPLE_RATE}");
    let config: cpal::StreamConfig = supported.config();
    let cb_buf = buf.clone();
    let rs = Arc::new(Mutex::new(Resampler::new(in_rate, SAMPLE_RATE)));
    // Running totals so we can log "still alive, N samples, peak X" once a second.
    let seen = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let err_fn = |e| eprintln!("[mic] stream err: {e}");

    // Android/emulator mics are usually i16, not f32 — handle every format (else we
    // capture nothing and the bot's audio track gets cleared).
    macro_rules! build {
        ($t:ty, $to_f32:expr) => {{
            let cb_buf = cb_buf.clone();
            let rs = rs.clone();
            let ml = mic_level.clone();
            let seen = seen.clone();
            device.build_input_stream(
                &config,
                move |data: &[$t], _: &cpal::InputCallbackInfo| {
                    let mono: Vec<f32> = if channels <= 1 {
                        data.iter().map(|&s| $to_f32(s)).collect()
                    } else {
                        data.chunks(channels).map(|f| $to_f32(f[0])).collect()
                    };
                    // Peak of this callback (pre-resample) → UI meter (keep the max).
                    let peak = mono.iter().fold(0f32, |a, &x| a.max(x.abs()));
                    {
                        let mut g = ml.lock().unwrap();
                        if peak > *g {
                            *g = peak;
                        }
                    }
                    seen.fetch_add(mono.len() as u64, Ordering::Relaxed);
                    let mut out = Vec::new();
                    rs.lock().unwrap().process(&mono, &mut out);
                    cb_buf.lock().unwrap().extend(out);
                },
                err_fn,
                None,
            )
        }};
    }
    let stream = match fmt {
        cpal::SampleFormat::F32 => build!(f32, |s: f32| s),
        cpal::SampleFormat::I16 => build!(i16, |s: i16| s as f32 / 32768.0),
        cpal::SampleFormat::U16 => build!(u16, |s: u16| (s as f32 - 32768.0) / 32768.0),
        other => {
            eprintln!("[mic] unsupported sample format {other:?} — cannot capture");
            return;
        }
    };
    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[mic] build_input_stream failed: {e}");
            return;
        }
    };
    if let Err(e) = stream.play() {
        eprintln!("[mic] stream.play() failed: {e}");
        return;
    }
    eprintln!("[mic] capturing…");
    // Keep the stream alive until the session stops; log a heartbeat once a second so
    // we can see whether callbacks are actually firing (silent mic = count stays 0).
    let mut last = 0u64;
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(1000));
        let n = seen.load(Ordering::Relaxed);
        let peak = *mic_level.lock().unwrap();
        eprintln!("[mic] +{} samples/s (total {n}), peak≈{peak:.3}", n - last);
        last = n;
    }
    drop(stream);
    eprintln!("[mic] stopped (captured {} samples total)", seen.load(Ordering::Relaxed));
}
