//! On-device TTS via sherpa-onnx on a dedicated audio thread, played via rodio.
//! Two engines to compare on-device:
//!   engine 0 = Supertonic-3 (English, flow-matching; sid = voice, num_steps = level)
//!   engine 1 = Piper VITS id_ID (Indonesian, single-pass; much faster)
//! `threads` = CPU cores. Streaming: each produced chunk is queued to rodio
//! immediately so playback starts on the first chunk. Reports 1st-audio + RTF.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc::{channel, Sender};

pub const SAMPLE_EN: &str =
    "Hello, I'm Kaira. This is the Supertonic voice running on device.";
pub const SAMPLE_ID: &str =
    "Halo, saya Kaira. Ini adalah suara yang berjalan langsung di perangkat Anda.";

pub const ENGINE_SUPERTONIC: u8 = 0;
pub const ENGINE_PIPER_ID: u8 = 1;
pub const ENGINE_SONIOX_ID: u8 = 2; // cloud (Soniox tts-rt-v1), Indonesian
pub const ENGINE_SONIOX_EN: u8 = 3; // cloud (Soniox tts-rt-v1), English
pub const ENGINE_MMS_ID: u8 = 4; // Meta MMS VITS id (16 kHz, char tokens, no espeak)

#[cfg(target_os = "android")]
pub enum Cmd {
    Speak {
        text: String,
        sid: i32,
        num_steps: i32,
        threads: i32,
        engine: u8,
    },
    /// Synthesize the same text with every compare engine, store each for replay.
    CompareAll { text: String },
    /// Replay a stored compare clip by index.
    PlayStored { idx: usize },
    /// Round-trip: run `user_text` through the cloud LLM, then deliver the reply.
    /// `opus_cap` 0 = on-device TTS; >0 = fetch the reply as an Opus stream from
    /// the TTS server at that simulated 3G downlink cap + `opus_bitrate`.
    RoundTrip { user_text: String, engine: u8, opus_cap: u32, opus_bitrate: u32 },
    /// Opus streaming test: connect to `host`, tell it the downlink cap + encode
    /// bitrate, decode + play the streamed Opus, and report whether it kept up.
    OpusStream { host: String, cap_kbps: u32, bitrate: u32 },
    /// Preload the on-device voice model into the cache (e.g. while STT runs) so
    /// the next round-trip synth skips the cold model load. No-op if already warm.
    Warmup { engine: u8 },
}

/// Result of a round-trip's LLM + TTS halves (STT half is timed on the UI side).
#[cfg(target_os = "android")]
#[derive(Default)]
pub struct RoundTripResult {
    pub reply: String,
    pub llm: f32,             // LLM wall-clock (s); 0 if no LLM ran
    pub tts_first: f32,       // TTS time-to-first-audio (s) — server synth+stream if Opus
    pub rebuffer_ms: f32,     // Opus-stream rebuffer (ms); 0 for on-device TTS
    pub streamed: bool,       // true if the reply streamed sentence-by-sentence
    pub wire_bytes: usize,    // bytes crossing the network to deliver the audio
    pub audio_over_wire: bool, // true = Opus audio on the wire; false = text only
}

/// Result of the in-app Opus streaming test (server streams Opus over a
/// bandwidth-capped link; device decodes + plays and measures if it kept up).
#[cfg(target_os = "android")]
#[derive(Default)]
pub struct OpusResult {
    pub first_audio: f32,  // connect → first decoded audio (s)
    pub audio_secs: f32,   // total audio decoded (s)
    pub late_packets: u32, // packets that missed their real-time playout deadline
    pub late_ms: f32,      // cumulative lateness (ms) = audible stall time
    pub recv_kbps: f32,    // effective received bitrate
    pub ok: bool,          // true = streamed smoothly (no late packets)
    pub err: String,
}

/// Per-engine result of a TTS compare (delivered to the UI as each finishes).
#[cfg(target_os = "android")]
pub struct TtsCompareResult {
    pub engine: String,
    pub first: f32, // time-to-first-audio (s)
    pub total: f32, // total synth wall-clock (s)
    pub dur: f32,   // audio duration (s)
}

/// Engines compared in TTS-compare mode — all speak the SAME Indonesian line.
#[cfg(target_os = "android")]
const TTS_COMPARE: [(u8, &str); 4] = [
    (ENGINE_SUPERTONIC, "Supertonic·ID"),
    (ENGINE_PIPER_ID, "Piper·ID"),
    (ENGINE_MMS_ID, "MMS·ID"),
    (ENGINE_SONIOX_ID, "Soniox·ID"),
];

/// A synthesized clip held for replay.
#[cfg(target_os = "android")]
struct Clip {
    samples: Vec<f32>,
    sr: u32,
}

#[cfg(target_os = "android")]
pub struct Tts {
    tx: Sender<Cmd>,
}

#[cfg(target_os = "android")]
impl Tts {
    pub fn new(
        report: Box<dyn Fn(String) + Send>,
        on_compare: Box<dyn Fn(TtsCompareResult) + Send>,
        on_roundtrip: Box<dyn Fn(RoundTripResult) + Send>,
        on_opus: Box<dyn Fn(OpusResult) + Send>,
        on_reply: Box<dyn Fn(String) + Send>,
    ) -> Self {
        let (tx, rx) = channel::<Cmd>();
        std::thread::spawn(move || {
            audio_thread(rx, report, on_compare, on_roundtrip, on_opus, on_reply)
        });
        Self { tx }
    }
    pub fn speak(&self, text: String, sid: i32, num_steps: i32, threads: i32, engine: u8) {
        let _ = self.tx.send(Cmd::Speak { text, sid, num_steps, threads, engine });
    }
    pub fn compare_all(&self, text: String) {
        let _ = self.tx.send(Cmd::CompareAll { text });
    }
    pub fn play_stored(&self, idx: usize) {
        let _ = self.tx.send(Cmd::PlayStored { idx });
    }
    /// Run `user_text` through the LLM, then speak the reply (round-trip test).
    /// `opus_cap` 0 = on-device TTS; >0 = server Opus stream at that 3G cap + bitrate.
    pub fn roundtrip_speak(&self, user_text: String, engine: u8, opus_cap: u32, opus_bitrate: u32) {
        let _ = self.tx.send(Cmd::RoundTrip { user_text, engine, opus_cap, opus_bitrate });
    }
    /// Start the Opus streaming test against `host` at a simulated cap + bitrate.
    pub fn opus_stream(&self, host: String, cap_kbps: u32, bitrate: u32) {
        let _ = self.tx.send(Cmd::OpusStream { host, cap_kbps, bitrate });
    }
    /// Preload the on-device voice model so the next round-trip synth is warm.
    pub fn warmup(&self, engine: u8) {
        let _ = self.tx.send(Cmd::Warmup { engine });
    }
}

pub(crate) fn files_dir() -> String {
    #[cfg(target_os = "android")]
    {
        // Shared storage (adb-writable + app-readable with storage permission),
        // works on both old Android and scoped-storage 11+.
        "/sdcard/kaira".to_string()
    }
    #[cfg(target_os = "ios")]
    {
        // The app-sandbox Documents dir — works on a REAL DEVICE and the simulator. iOS sets
        // HOME to the app's container, so $HOME/Documents is the standard writable location.
        // (Previously this returned a dev host path that ONLY the simulator could read, so
        // settings persistence + the realtime_url override silently failed on every device.)
        std::env::var("HOME")
            .map(|h| format!("{h}/Documents"))
            .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned())
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        // Desktop-dev convenience (VOIDAI_DEMO runs read seed/config from here).
        "/Users/jek/Documents/Projects/Personal/Rust-Mobile/voicelab".to_string()
    }
}

/// Lean server base URL (Kaira backend on fly) — mints STT tokens, serves LLM/TTS.
/// From `<files_dir>/lean_server.txt`, defaults to the deployed micro-agent-lean.
pub(crate) fn lean_server() -> String {
    std::fs::read_to_string(format!("{}/lean_server.txt", files_dir()))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://micro-agent-lean.fly.dev".to_string())
}

/// Soniox API key, read from `<files_dir>/soniox_key.txt` (or $SONIOX_API_KEY on host).
pub(crate) fn soniox_key() -> Option<String> {
    if let Ok(k) = std::env::var("SONIOX_API_KEY") {
        if !k.trim().is_empty() {
            return Some(k.trim().to_string());
        }
    }
    std::fs::read_to_string(format!("{}/soniox_key.txt", files_dir()))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The default output device's sample rate (what rodio's mixer runs at). We
/// pre-resample every clip to this so rodio never has to — its per-source
/// resampling of odd rates (22050/24000/44100) is fast+glitchy on some Android
/// audio HALs (esp. the emulator).
fn default_output_rate() -> u32 {
    // Android's AAudio output runs at 48 kHz on effectively every device (incl.
    // the emulator); querying cpal's default_output_config() there can abort in
    // the audio HAL, so hardcode it and only probe cpal on desktop.
    #[cfg(target_os = "android")]
    {
        48_000
    }
    #[cfg(not(target_os = "android"))]
    {
        use cpal::traits::{DeviceTrait, HostTrait};
        cpal::default_host()
            .default_output_device()
            .and_then(|d| d.default_output_config().ok())
            .map(|c| c.sample_rate().0)
            .unwrap_or(48_000)
    }
}

/// Continuous linear resampler — carries fractional state across chunks so
/// streaming produces no boundary clicks. (Our targets are always upsampling.)
pub(crate) struct Resampler {
    step: f64, // input samples advanced per output sample (from/to)
    frac: f64,
    last: f32,
}
impl Resampler {
    pub(crate) fn new(from: u32, to: u32) -> Self {
        Self { step: from as f64 / to as f64, frac: 0.0, last: 0.0 }
    }
    pub(crate) fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.step <= 0.0 {
            out.extend_from_slice(input);
            return;
        }
        for &x in input {
            while self.frac < 1.0 {
                out.push(self.last + (x - self.last) * self.frac as f32);
                self.frac += self.step;
            }
            self.frac -= 1.0;
            self.last = x;
        }
    }
    fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
        if from == to {
            return input.to_vec();
        }
        let mut rs = Resampler::new(from, to);
        let mut out = Vec::with_capacity(input.len() * to as usize / from.max(1) as usize + 8);
        rs.process(input, &mut out);
        out
    }
}

#[cfg(target_os = "android")]
fn audio_thread(
    rx: std::sync::mpsc::Receiver<Cmd>,
    report: Box<dyn Fn(String) + Send>,
    on_compare: Box<dyn Fn(TtsCompareResult) + Send>,
    on_roundtrip: Box<dyn Fn(RoundTripResult) + Send>,
    on_opus: Box<dyn Fn(OpusResult) + Send>,
    on_reply: Box<dyn Fn(String) + Send>,
) {
    let out_sr = default_output_rate();
    let mut audio: Option<(rodio::OutputStream, rodio::OutputStreamHandle)> = None;
    let mut engine: Option<sherpa_onnx::OfflineTts> = None;
    let mut cached: (u8, i32) = (255, 0); // (engine, threads) currently loaded
    let mut sink: Option<Rc<rodio::Sink>> = None;
    let mut rt: Option<tokio::runtime::Runtime> = None; // lazy tokio for Soniox
    // One reused HTTP client → keep-alive to the lean server, so round-trips after
    // the first skip the DNS+TCP+TLS handshake (~0.3s) to fly.dev. Force HTTP/1.1:
    // on this current-thread tokio runtime, HTTP/2's separate connection-driver task
    // isn't polled promptly during block_on, which STALLS the streamed /chat/stream
    // body by seconds (curl on the same device gets the first byte in 0.16s). HTTP/1.1
    // streams the body directly on the response future — no driver-task dependency.
    let mut http: Option<reqwest::Client> = None;
    let mut stored: Vec<Clip> = Vec::new(); // synthesized compare clips, for replay

    while let Ok(cmd) = rx.recv() {
        // Ensure audio output (every path needs it).
        if audio.is_none() {
            match rodio::OutputStream::try_default() {
                Ok(v) => audio = Some(v),
                Err(e) => {
                    report(format!("no audio output: {e}"));
                    continue;
                }
            }
        }

        // Round-trip LLM timing + reply, carried to the on_roundtrip callback.
        let mut rt_llm_secs = 0f32;
        let mut rt_reply = String::new();

        // Compare commands are handled here; Speak/RoundTrip fall through to the synth body.
        let (text, sid, num_steps, threads, eng, is_rt) = match cmd {
            Cmd::Speak { text, sid, num_steps, threads, engine } => {
                (text, sid, num_steps, threads, engine, false)
            }
            Cmd::RoundTrip { user_text, engine: tts_eng, opus_cap, opus_bitrate } => {
                if rt.is_none() {
                    rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok();
                }
                let Some(runtime) = rt.as_ref() else {
                    report("LLM: runtime init failed".into());
                    continue;
                };
                let client = http
                    .get_or_insert_with(|| {
                        reqwest::Client::builder()
                            .http1_only()
                            // Drop idle connections after 8s so we never REUSE one that
                            // fly already closed during a between-turn gap — a stale
                            // keep-alive reuse stalls the request for seconds. TCP
                            // keepalive also nudges the socket so it stays healthy.
                            .pool_idle_timeout(std::time::Duration::from_secs(8))
                            .tcp_keepalive(std::time::Duration::from_secs(10))
                            .build()
                            .unwrap_or_default()
                    })
                    .clone();
                let lean = lean_server();
                let lang = if tts_eng == ENGINE_PIPER_ID || tts_eng == ENGINE_SONIOX_ID {
                    1u32 // Indonesian
                } else {
                    0u32 // English
                };

                if opus_cap > 0 {
                    // STREAMING server path: /chat/stream → per-sentence fly /tts,
                    // pipelined so sentence 1 plays while the LLM writes sentence 2.
                    if let Some(s) = sink.take() {
                        s.stop();
                    }
                    let handle = &audio.as_ref().unwrap().1;
                    let sk = match rodio::Sink::try_new(handle) {
                        Ok(s) => Rc::new(s),
                        Err(e) => {
                            report(format!("audio: {e}"));
                            continue;
                        }
                    };
                    report(format!("reply · streaming · fly Opus {opus_bitrate}k…"));
                    let res = runtime.block_on(async {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(60),
                            stream_roundtrip(&client, &lean, &user_text, opus_bitrate, lang, &sk, &on_reply),
                        )
                        .await
                        {
                            Ok(r) => r,
                            Err(_) => StreamResult { err: "timed out (60s)".into(), ..Default::default() },
                        }
                    });
                    sink = Some(sk);
                    if !res.err.is_empty() {
                        report(format!("stream error: {}", res.err));
                    }
                    on_roundtrip(RoundTripResult {
                        reply: res.reply,
                        llm: res.llm_first,
                        tts_first: res.tts_first,
                        rebuffer_ms: 0.0,
                        streamed: true,
                        wire_bytes: res.wire_bytes,
                        audio_over_wire: !res.text_only,
                    });
                    continue;
                }

                // ON-DEVICE streaming path — only the reply TEXT crosses the network;
                // each sentence is synthesized on-device (Piper/Supertonic) as it
                // arrives. Smallest possible 3G payload (~text) + instant local synth.
                if engine.is_none() || cached != (tts_eng, 4) {
                    report("loading on-device voice…".into());
                    engine = create_engine(tts_eng, 4, &report);
                    cached = (tts_eng, 4);
                }
                let Some(e) = engine.as_ref() else {
                    report("on-device voice failed to load".into());
                    continue;
                };
                if let Some(s) = sink.take() {
                    s.stop();
                }
                let handle = &audio.as_ref().unwrap().1;
                let sk = match rodio::Sink::try_new(handle) {
                    Ok(s) => Rc::new(s),
                    Err(err) => {
                        report(format!("audio: {err}"));
                        continue;
                    }
                };
                report("reply · streaming · on-device TTS (text-only)…".into());
                let sk2 = sk.clone();
                let res = runtime.block_on(async {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(60),
                        stream_roundtrip_text(&client, &lean, &user_text, lang, &on_reply, move |sentence: &str| {
                            // Stream each chunk straight to the sink → audio starts at
                            // the first chunk, not after the whole sentence synthesizes.
                            synth_sentence_streaming(e, tts_eng, sentence, lang, out_sr, &sk2);
                        }),
                    )
                    .await
                    {
                        Ok(r) => r,
                        Err(_) => StreamResult { err: "timed out (60s)".into(), ..Default::default() },
                    }
                });
                sink = Some(sk);
                if !res.err.is_empty() {
                    report(format!("stream error: {}", res.err));
                }
                on_roundtrip(RoundTripResult {
                    reply: res.reply,
                    llm: res.llm_first,
                    tts_first: res.tts_first,
                    rebuffer_ms: 0.0,
                    streamed: true,
                    wire_bytes: res.wire_bytes,
                    audio_over_wire: !res.text_only,
                });
                continue;
            }
            Cmd::CompareAll { text } => {
                stored.clear();
                if let Some(s) = sink.take() {
                    s.stop();
                }
                let handle = &audio.as_ref().unwrap().1;
                for (i, (e2, label)) in TTS_COMPARE.iter().enumerate() {
                    report(format!("compare [{}/{}] {label} · synthesizing…", i + 1, TTS_COMPARE.len()));
                    if let Some((samples, sr, first, tot)) =
                        synth_to_buffer(*e2, &text, 4, &mut rt, &report, out_sr)
                    {
                        let dur = samples.len() as f32 / sr as f32;
                        on_compare(TtsCompareResult { engine: label.to_string(), first, total: tot, dur });
                        // Auto-play from the first sample, blocking until it ends so the
                        // engines play back-to-back (no overlap) for a clean A/B listen.
                        if let Ok(sk) = rodio::Sink::try_new(handle) {
                            sk.append(rodio::buffer::SamplesBuffer::new(1u16, sr, samples.clone()));
                            report(format!("compare [{}/{}] {label} · ▶ playing…", i + 1, TTS_COMPARE.len()));
                            sk.sleep_until_end();
                        }
                        stored.push(Clip { samples, sr });
                    }
                }
                report("compare done — tap ▶ to replay each".to_string());
                continue;
            }
            Cmd::PlayStored { idx } => {
                if let Some(clip) = stored.get(idx) {
                    if let Some(s) = sink.take() {
                        s.stop();
                    }
                    let handle = &audio.as_ref().unwrap().1;
                    if let Ok(sk) = rodio::Sink::try_new(handle) {
                        sk.append(rodio::buffer::SamplesBuffer::new(1u16, clip.sr, clip.samples.clone()));
                        sink = Some(Rc::new(sk));
                    }
                }
                continue;
            }
            Cmd::OpusStream { host, cap_kbps, bitrate } => {
                if rt.is_none() {
                    rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok();
                }
                let Some(runtime) = rt.as_ref() else {
                    on_opus(OpusResult { err: "runtime init failed".into(), ..Default::default() });
                    continue;
                };
                if let Some(s) = sink.take() {
                    s.stop();
                }
                let handle = &audio.as_ref().unwrap().1;
                let sk = match rodio::Sink::try_new(handle) {
                    Ok(s) => Rc::new(s),
                    Err(e) => {
                        on_opus(OpusResult { err: format!("audio: {e}"), ..Default::default() });
                        continue;
                    }
                };
                report(format!("Opus stream · {host} · {bitrate}k @ {cap_kbps}k cap…"));
                // Hard 30s ceiling so a slow/hung link can never block the thread forever.
                let result = runtime.block_on(async {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        stream_opus(&host, cap_kbps, bitrate, &sk),
                    )
                    .await
                    {
                        Ok(r) => r,
                        Err(_) => OpusResult { err: "timed out (30s)".into(), ..Default::default() },
                    }
                });
                sink = Some(sk); // keep playing after the stream ends
                on_opus(result);
                continue;
            }
            Cmd::Warmup { engine: warm_eng } => {
                // Load the model now (typically during STT) so the first synth
                // isn't cold. Silent — don't clobber the STT status. Cached after.
                if engine.is_none() || cached != (warm_eng, 4) {
                    engine = create_engine(warm_eng, 4, &report);
                    cached = (warm_eng, 4);
                }
                continue;
            }
        };

        let handle = &audio.as_ref().unwrap().1;

        // Fresh sink + timing for this utterance (both engine paths use them).
        if let Some(s) = sink.take() {
            s.stop();
        }
        let stream_sink = match rodio::Sink::try_new(handle) {
            Ok(s) => Rc::new(s),
            Err(err) => {
                report(format!("audio: {err}"));
                continue;
            }
        };
        let started = std::time::Instant::now();
        let ttfa = Rc::new(Cell::new(0.0f32));
        let total = Rc::new(Cell::new(0usize));

        // ---- Soniox cloud engine (streams audio in over WebSocket) ----
        if eng == ENGINE_SONIOX_ID || eng == ENGINE_SONIOX_EN {
            let lang = if eng == ENGINE_SONIOX_EN { "en" } else { "id" };
            let key = match soniox_key() {
                Some(k) => k,
                None => {
                    report("Soniox key missing — put it in /sdcard/kaira/soniox_key.txt".into());
                    continue;
                }
            };
            report("Soniox · connecting…".to_string());
            if rt.is_none() {
                rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .ok();
            }
            let Some(runtime) = rt.as_ref() else {
                report("Soniox: runtime init failed".into());
                continue;
            };
            let cb_sink = stream_sink.clone();
            let cb_ttfa = ttfa.clone();
            let cb_total = total.clone();
            let t0 = started;
            let mut rs = Resampler::new(crate::soniox::SAMPLE_RATE, out_sr);
            let res = runtime.block_on(crate::soniox::synth(
                &key,
                &text,
                lang,
                "Maya",
                move |chunk: &[f32]| {
                    if cb_total.get() == 0 {
                        cb_ttfa.set(t0.elapsed().as_secs_f32());
                    }
                    cb_total.set(cb_total.get() + chunk.len());
                    let mut resampled = Vec::new();
                    rs.process(chunk, &mut resampled);
                    if !resampled.is_empty() {
                        cb_sink.append(rodio::buffer::SamplesBuffer::new(1u16, out_sr, resampled));
                    }
                },
            ));
            match res {
                Ok(()) if total.get() > 0 => {
                    let dur = total.get() as f32 / crate::soniox::SAMPLE_RATE as f32;
                    let net = started.elapsed().as_secs_f32();
                    report(format!(
                        "Soniox·{lang} · 1st {:.2}s · net {net:.1}s · {dur:.1}s  ▶",
                        ttfa.get()
                    ));
                    sink = Some(stream_sink);
                }
                Ok(()) => report("Soniox: no audio returned".into()),
                Err(e) => report(format!("Soniox error: {e}")),
            }
            if is_rt {
                on_roundtrip(RoundTripResult {
                    reply: rt_reply.clone(),
                    llm: rt_llm_secs,
                    tts_first: ttfa.get(),
                    rebuffer_ms: 0.0,
                    streamed: false,
                    ..Default::default()
                });
            }
            continue;
        }

        // ---- On-device sherpa engines (Supertonic / Piper) ----
        if engine.is_none() || cached != (eng, threads) {
            let name = if eng == ENGINE_PIPER_ID { "Piper id" } else { "Supertonic" };
            report(format!("loading {name} · {threads} thread(s)…"));
            engine = create_engine(eng, threads, &report);
            cached = (eng, threads);
        }
        let Some(e) = engine.as_ref() else { continue };
        let sr = e.sample_rate() as u32;

        {
            use sherpa_onnx::GenerationConfig;
            // Supertonic: sid = voice, num_steps = level, lang tag. Piper: single
            // speaker (sid 0), no steps, language is baked into the model.
            let cfg = if eng == ENGINE_PIPER_ID || eng == ENGINE_MMS_ID {
                GenerationConfig { sid: 0, speed: 1.0, ..Default::default() }
            } else {
                let mut extra = std::collections::HashMap::new();
                extra.insert("lang".to_string(), serde_json::json!("en"));
                GenerationConfig { sid, num_steps, speed: 1.0, extra: Some(extra), ..Default::default() }
            };
            let cb_sink = stream_sink.clone();
            let cb_ttfa = ttfa.clone();
            let cb_total = total.clone();
            let t0 = started;
            let mut rs = Resampler::new(sr, out_sr);
            let cb = move |chunk: &[f32], _p: f32| -> bool {
                if cb_total.get() == 0 {
                    cb_ttfa.set(t0.elapsed().as_secs_f32());
                }
                cb_total.set(cb_total.get() + chunk.len());
                let mut res = Vec::new();
                rs.process(chunk, &mut res);
                if !res.is_empty() {
                    cb_sink.append(rodio::buffer::SamplesBuffer::new(1u16, out_sr, res));
                }
                true
            };
            e.generate_with_config(&text, &cfg, Some(cb));
        }

        let n = total.get();
        if n == 0 {
            report("generation failed".to_string());
            continue;
        }
        let gen = started.elapsed().as_secs_f32();
        let dur = n as f32 / sr as f32;
        let rtf = gen / dur.max(0.001);
        let name = match eng {
            ENGINE_PIPER_ID => "Piper-id",
            ENGINE_MMS_ID => "MMS-id",
            _ => "Supertonic",
        };
        report(format!(
            "{name} · {threads}thr · 1st {:.2}s · RTF {rtf:.2} · {dur:.1}s  ▶",
            ttfa.get()
        ));
        sink = Some(stream_sink);
        if is_rt {
            on_roundtrip(RoundTripResult {
                reply: rt_reply.clone(),
                llm: rt_llm_secs,
                tts_first: ttfa.get(),
                rebuffer_ms: 0.0,
                streamed: false,
                ..Default::default()
            });
        }
    }
}

/// Synthesize `text` with one engine into an in-memory buffer (no live playback),
/// measuring time-to-first-audio + total. Used by TTS compare mode.
#[cfg(target_os = "android")]
fn synth_to_buffer(
    engine: u8,
    text: &str,
    threads: i32,
    rt: &mut Option<tokio::runtime::Runtime>,
    report: &dyn Fn(String),
    out_sr: u32,
) -> Option<(Vec<f32>, u32, f32, f32)> {
    let started = std::time::Instant::now();
    let first = Rc::new(Cell::new(0f32));
    let acc = Rc::new(std::cell::RefCell::new(Vec::<f32>::new()));

    if engine == ENGINE_SONIOX_ID || engine == ENGINE_SONIOX_EN {
        let lang = if engine == ENGINE_SONIOX_EN { "en" } else { "id" };
        let key = match soniox_key() {
            Some(k) => k,
            None => {
                report("Soniox key missing — /sdcard/kaira/soniox_key.txt".to_string());
                return None;
            }
        };
        if rt.is_none() {
            *rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok();
        }
        let runtime = rt.as_ref()?;
        let f = first.clone();
        let a = acc.clone();
        let t0 = started;
        let res = runtime.block_on(crate::soniox::synth(&key, text, lang, "Maya", move |chunk: &[f32]| {
            if a.borrow().is_empty() {
                f.set(t0.elapsed().as_secs_f32());
            }
            a.borrow_mut().extend_from_slice(chunk);
        }));
        if let Err(e) = res {
            report(format!("Soniox error: {e}"));
            return None;
        }
        let samples = Resampler::resample(&acc.borrow(), crate::soniox::SAMPLE_RATE, out_sr);
        Some((samples, out_sr, first.get(), started.elapsed().as_secs_f32()))
    } else {
        let e = create_engine(engine, threads, report)?;
        let sr = e.sample_rate() as u32;
        use sherpa_onnx::GenerationConfig;
        let cfg = if engine == ENGINE_PIPER_ID || engine == ENGINE_MMS_ID {
            GenerationConfig { sid: 0, speed: 1.0, ..Default::default() }
        } else {
            let mut extra = std::collections::HashMap::new();
            extra.insert("lang".to_string(), serde_json::json!("id")); // Supertonic → Indonesian
            GenerationConfig { sid: 5, num_steps: 6, speed: 1.0, extra: Some(extra), ..Default::default() }
        };
        let f = first.clone();
        let a = acc.clone();
        let t0 = started;
        let cb = move |chunk: &[f32], _p: f32| -> bool {
            if a.borrow().is_empty() {
                f.set(t0.elapsed().as_secs_f32());
            }
            a.borrow_mut().extend_from_slice(chunk);
            true
        };
        e.generate_with_config(text, &cfg, Some(cb));
        let samples = Resampler::resample(&acc.borrow(), sr, out_sr);
        Some((samples, out_sr, first.get(), started.elapsed().as_secs_f32()))
    }
}

/// Connect to the test server, tell it the simulated 3G downlink cap, then read
/// length-prefixed Opus packets (`[u32 LE len][packet]`, len 0 = EOF), decode
/// each (48 kHz native) and play it, measuring whether each 20 ms packet lands
/// before its real-time playout deadline (with a 60 ms jitter buffer).
#[cfg(target_os = "android")]
async fn stream_opus(host: &str, cap_kbps: u32, bitrate: u32, sink: &rodio::Sink) -> OpusResult {
    use tokio::io::AsyncWriteExt;
    let mut r = OpusResult::default();
    let mut stream = match tokio::net::TcpStream::connect(host).await {
        Ok(s) => s,
        Err(e) => {
            r.err = format!("connect {host}: {e}");
            return r;
        }
    };
    let mut hdr = [0u8; 8];
    hdr[..4].copy_from_slice(&cap_kbps.to_le_bytes());
    hdr[4..].copy_from_slice(&bitrate.to_le_bytes());
    if let Err(e) = stream.write_all(&hdr).await {
        r.err = format!("handshake: {e}");
        return r;
    }
    recv_and_play(&mut stream, sink).await
}

/// Result of the STREAMING round-trip.
#[cfg(target_os = "android")]
#[derive(Default)]
struct StreamResult {
    reply: String,
    llm_first: f32,     // POST → first `say` sentence from the LLM (s)
    tts_first: f32,     // POST → first sentence's audio queued (s) = time-to-first-audio
    wire_bytes: usize,  // bytes that crossed the network to deliver the audio
    text_only: bool,    // true = on-device synth (only text on the wire)
    err: String,
}

/// Streaming + pipelined round-trip: POST the transcript to the lean `/chat/stream`
/// (NDJSON), and as each `{"type":"say"}` sentence arrives, synthesize it via the
/// fly `/tts` (Opus) and queue it. So sentence 1's audio plays while the LLM is
/// still writing sentence 2 — time-to-first-audio drops to first-sentence latency
/// instead of waiting for the whole reply.
#[cfg(target_os = "android")]
async fn stream_roundtrip(
    client: &reqwest::Client,
    lean: &str,
    user_text: &str,
    bitrate: u32,
    lang: u32,
    sink: &rodio::Sink,
    on_reply: &dyn Fn(String),
) -> StreamResult {
    let mut r = StreamResult::default();
    let base = lean.trim_end_matches('/');
    let chat_url = format!("{base}/chat/stream");
    let tts_url = format!("{base}/tts");
    let langstr = if lang == 1 { "id" } else { "en" };
    let body = serde_json::json!({
        "messages": [{ "role": "user", "content": user_text }],
        "language": langstr,
    });
    let t0 = std::time::Instant::now();
    let mut resp = match client.post(&chat_url).json(&body).send().await {
        Ok(x) => x,
        Err(e) => {
            r.err = format!("chat/stream: {e}");
            return r;
        }
    };
    eprintln!("[rt] server: /chat/stream connected in {:.2}s", t0.elapsed().as_secs_f32());
    if !resp.status().is_success() {
        r.err = format!("chat/stream HTTP {}", resp.status().as_u16());
        return r;
    }
    let mut buf: Vec<u8> = Vec::new();
    let mut first_chunk = false;
    'outer: loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                if r.err.is_empty() {
                    r.err = format!("stream: {e}");
                }
                break;
            }
        };
        if !first_chunk {
            first_chunk = true;
            eprintln!("[rt] server: 1st body chunk at {:.2}s ({} B)", t0.elapsed().as_secs_f32(), chunk.len());
        }
        buf.extend_from_slice(&chunk);
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let line = &line[..line.len().saturating_sub(1)];
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_slice(line) {
                Ok(x) => x,
                Err(_) => continue,
            };
            match v["type"].as_str() {
                Some("say") => {
                    let sentence = crate::llm::normalize(v["text"].as_str().unwrap_or(""));
                    if sentence.trim().is_empty() {
                        continue;
                    }
                    if r.llm_first == 0.0 {
                        r.llm_first = t0.elapsed().as_secs_f32();
                        eprintln!("[rt] server: LLM 1st sentence in {:.2}s: {:?}", r.llm_first, sentence);
                    }
                    if !r.reply.is_empty() {
                        r.reply.push(' ');
                    }
                    r.reply.push_str(&sentence);
                    on_reply(r.reply.clone()); // stream the reply text to the UI now
                    // Synthesize this sentence via fly /tts and STREAM the Opus back:
                    // decode + queue each length-prefixed packet as it arrives, so audio
                    // starts at the first ~20ms packet instead of the whole clause.
                    let tbody = serde_json::json!({ "text": sentence, "lang": langstr, "format": "opus", "bitrate": bitrate });
                    let tt = std::time::Instant::now();
                    if let (Ok(mut tresp), Some(mut dec)) =
                        (client.post(&tts_url).json(&tbody).send().await, crate::opus::Decoder::new())
                    {
                        if tresp.status().is_success() {
                            let mut tbuf: Vec<u8> = Vec::new();
                            'tts: loop {
                                match tresp.chunk().await {
                                    Ok(Some(c)) => {
                                        r.wire_bytes += c.len(); // Opus audio over the wire
                                        tbuf.extend_from_slice(&c);
                                    }
                                    _ => break, // stream end or error
                                }
                                // Decode every COMPLETE [u32 len][pkt] now buffered.
                                let mut off = 0usize;
                                loop {
                                    if off + 4 > tbuf.len() {
                                        break;
                                    }
                                    let len = u32::from_le_bytes([tbuf[off], tbuf[off + 1], tbuf[off + 2], tbuf[off + 3]]) as usize;
                                    if len == 0 {
                                        break 'tts; // EOF marker
                                    }
                                    if off + 4 + len > tbuf.len() {
                                        break; // packet not fully arrived yet
                                    }
                                    let mut pcm = Vec::new();
                                    if dec.decode(&tbuf[off + 4..off + 4 + len], &mut pcm) > 0 {
                                        sink.append(rodio::buffer::SamplesBuffer::new(1u16, crate::opus::SAMPLE_RATE, pcm));
                                        if r.tts_first == 0.0 {
                                            r.tts_first = t0.elapsed().as_secs_f32();
                                            eprintln!("[rt] server: 1st audio at {:.2}s (fly /tts 1st packet {:.2}s)", r.tts_first, tt.elapsed().as_secs_f32());
                                        }
                                    }
                                    off += 4 + len;
                                }
                                tbuf.drain(0..off);
                            }
                        }
                    }
                }
                Some("done") => break 'outer,
                Some("error") => {
                    r.err = v["message"].as_str().unwrap_or("stream error").to_string();
                    break 'outer;
                }
                _ => {}
            }
        }
    }
    r.reply = r.reply.trim().to_string();
    r
}

/// Shared receive loop: read length-prefixed Opus packets (`[u32 LE len][pkt]`,
/// len 0 = EOF), decode (48 kHz native) + play each, and measure first-audio +
/// rebuffer (stall time) + late-packet count against a 60 ms jitter buffer.
#[cfg(target_os = "android")]
async fn recv_and_play(stream: &mut tokio::net::TcpStream, sink: &rodio::Sink) -> OpusResult {
    use tokio::io::AsyncReadExt;
    let mut r = OpusResult::default();
    let mut dec = match crate::opus::Decoder::new() {
        Some(d) => d,
        None => {
            r.err = "opus decoder init failed".into();
            return r;
        }
    };
    const JITTER: f32 = 0.060;
    let t0 = std::time::Instant::now();
    let (mut pkts, mut samples, mut late, mut bytes) = (0u32, 0usize, 0u32, 0usize);
    let mut first_audio = 0f32;
    loop {
        let mut lenb = [0u8; 4];
        if stream.read_exact(&mut lenb).await.is_err() {
            break;
        }
        let len = u32::from_le_bytes(lenb) as usize;
        if len == 0 {
            break; // EOF
        }
        if len > 4000 {
            r.err = "bad frame length".into();
            break;
        }
        let mut pkt = vec![0u8; len];
        if stream.read_exact(&mut pkt).await.is_err() {
            break;
        }
        bytes += len + 4;
        let mut pcm = Vec::new();
        if dec.decode(&pkt, &mut pcm) <= 0 {
            continue;
        }
        let now = t0.elapsed().as_secs_f32();
        if pkts == 0 {
            first_audio = now;
        }
        let deadline = first_audio + JITTER + pkts as f32 * 0.020;
        if now > deadline {
            late += 1;
        }
        samples += pcm.len();
        sink.append(rodio::buffer::SamplesBuffer::new(1u16, crate::opus::SAMPLE_RATE, pcm));
        pkts += 1;
    }
    let elapsed = t0.elapsed().as_secs_f32().max(0.001);
    r.first_audio = first_audio;
    r.audio_secs = samples as f32 / crate::opus::SAMPLE_RATE as f32;
    r.late_packets = late;
    r.late_ms = (elapsed - first_audio - r.audio_secs).max(0.0) * 1000.0;
    r.recv_kbps = bytes as f32 * 8.0 / 1000.0 / elapsed;
    if pkts == 0 && r.err.is_empty() {
        r.err = "no packets received".into();
    }
    r.ok = r.err.is_empty() && pkts > 0 && r.late_ms < 150.0;
    r
}

/// Synthesize one sentence on-device to `out_sr` samples (synchronous, ~0.1s for
/// Piper). Used by the text-only streaming path.
/// Synthesize `text` on-device and stream each chunk straight to `sink` as it's
/// produced (stateful resample carried across chunks) — so the reply starts
/// playing at the FIRST chunk (~0.2s) instead of waiting for the whole sentence.
#[cfg(target_os = "android")]
fn synth_sentence_streaming(
    e: &sherpa_onnx::OfflineTts,
    engine_idx: u8,
    text: &str,
    lang: u32,
    out_sr: u32,
    sink: &Rc<rodio::Sink>,
) {
    use sherpa_onnx::GenerationConfig;
    let cfg = if engine_idx == ENGINE_PIPER_ID || engine_idx == ENGINE_MMS_ID {
        GenerationConfig { sid: 0, speed: 1.0, ..Default::default() }
    } else {
        let mut extra = std::collections::HashMap::new();
        extra.insert("lang".to_string(), serde_json::json!(if lang == 1 { "id" } else { "en" }));
        GenerationConfig { sid: 5, num_steps: 6, speed: 1.0, extra: Some(extra), ..Default::default() }
    };
    let sr = e.sample_rate() as u32;
    let sk = sink.clone();
    let rs = Rc::new(std::cell::RefCell::new(Resampler::new(sr, out_sr)));
    e.generate_with_config(text, &cfg, Some(move |chunk: &[f32], _p: f32| {
        let mut res = Vec::new();
        rs.borrow_mut().process(chunk, &mut res);
        if !res.is_empty() {
            sk.append(rodio::buffer::SamplesBuffer::new(1u16, out_sr, res));
        }
        true
    }));
}

/// Text-only streaming round-trip: read the lean `/chat/stream` sentences and, for
/// each one, call `synth` (on-device TTS) — so ONLY the reply TEXT crosses the
/// network (~100 bytes/sentence vs KB of Opus), yet audio still streams sentence
/// by sentence. The strongest 3G play: nothing but text on the wire.
#[cfg(target_os = "android")]
async fn stream_roundtrip_text(
    client: &reqwest::Client,
    lean: &str,
    user_text: &str,
    lang: u32,
    on_reply: &dyn Fn(String),
    mut synth: impl FnMut(&str),
) -> StreamResult {
    let mut r = StreamResult { text_only: true, ..Default::default() };
    let url = format!("{}/chat/stream", lean.trim_end_matches('/'));
    let langstr = if lang == 1 { "id" } else { "en" };
    let body = serde_json::json!({
        "messages": [{ "role": "user", "content": user_text }],
        "language": langstr,
    });
    let t0 = std::time::Instant::now();
    let mut resp = match client.post(&url).json(&body).send().await {
        Ok(x) => x,
        Err(e) => {
            r.err = format!("chat/stream: {e}");
            return r;
        }
    };
    eprintln!("[rt] on-device: /chat/stream connected in {:.2}s", t0.elapsed().as_secs_f32());
    if !resp.status().is_success() {
        r.err = format!("chat/stream HTTP {}", resp.status().as_u16());
        return r;
    }
    let mut buf: Vec<u8> = Vec::new();
    let mut first_chunk = false;
    'outer: loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                if r.err.is_empty() {
                    r.err = format!("stream: {e}");
                }
                break;
            }
        };
        if !first_chunk {
            first_chunk = true;
            eprintln!("[rt] on-device: 1st body chunk at {:.2}s ({} B)", t0.elapsed().as_secs_f32(), chunk.len());
        }
        buf.extend_from_slice(&chunk);
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let line = &line[..line.len().saturating_sub(1)];
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_slice(line) {
                Ok(x) => x,
                Err(_) => continue,
            };
            match v["type"].as_str() {
                Some("say") => {
                    let sentence = crate::llm::normalize(v["text"].as_str().unwrap_or(""));
                    if sentence.trim().is_empty() {
                        continue;
                    }
                    if r.llm_first == 0.0 {
                        r.llm_first = t0.elapsed().as_secs_f32();
                        eprintln!("[rt] on-device: LLM 1st sentence in {:.2}s: {:?}", r.llm_first, sentence);
                    }
                    if !r.reply.is_empty() {
                        r.reply.push(' ');
                    }
                    r.reply.push_str(&sentence);
                    on_reply(r.reply.clone()); // stream the reply text to the UI now
                    r.wire_bytes += sentence.len(); // only TEXT crosses the wire
                    let ts = std::time::Instant::now();
                    synth(&sentence); // on-device TTS + queue
                    if r.tts_first == 0.0 {
                        r.tts_first = t0.elapsed().as_secs_f32();
                        eprintln!("[rt] on-device: 1st audio queued at {:.2}s (synth {:.2}s)", r.tts_first, ts.elapsed().as_secs_f32());
                    }
                }
                Some("done") => break 'outer,
                Some("error") => {
                    r.err = v["message"].as_str().unwrap_or("stream error").to_string();
                    break 'outer;
                }
                _ => {}
            }
        }
    }
    r.reply = r.reply.trim().to_string();
    r
}

#[cfg(target_os = "android")]
fn create_engine(engine: u8, num_threads: i32, report: &dyn Fn(String)) -> Option<sherpa_onnx::OfflineTts> {
    use sherpa_onnx::{
        OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig, OfflineTtsSupertonicModelConfig,
        OfflineTtsVitsModelConfig,
    };
    let base = files_dir();
    let mut model_config = OfflineTtsModelConfig {
        num_threads: num_threads.max(1),
        debug: false,
        ..Default::default()
    };
    let dir = if engine == ENGINE_MMS_ID {
        // Meta MMS VITS: character tokens baked into tokens.txt, no espeak data_dir.
        let d = format!("{base}/vits-mms-ind");
        model_config.vits = OfflineTtsVitsModelConfig {
            model: Some(format!("{d}/model.onnx")),
            tokens: Some(format!("{d}/tokens.txt")),
            ..Default::default()
        };
        d
    } else if engine == ENGINE_PIPER_ID {
        let d = format!("{base}/vits-piper-id_ID-news_tts-medium-int8");
        model_config.vits = OfflineTtsVitsModelConfig {
            model: Some(format!("{d}/id_ID-news_tts-medium.onnx")),
            tokens: Some(format!("{d}/tokens.txt")),
            data_dir: Some(format!("{d}/espeak-ng-data")),
            ..Default::default()
        };
        d
    } else {
        let d = format!("{base}/sherpa-onnx-supertonic-3-tts-int8-2026-05-11");
        let p = |f: &str| Some(format!("{d}/{f}"));
        model_config.supertonic = OfflineTtsSupertonicModelConfig {
            duration_predictor: p("duration_predictor.int8.onnx"),
            text_encoder: p("text_encoder.int8.onnx"),
            vector_estimator: p("vector_estimator.int8.onnx"),
            vocoder: p("vocoder.int8.onnx"),
            tts_json: p("tts.json"),
            unicode_indexer: p("unicode_indexer.bin"),
            voice_style: p("voice.bin"),
        };
        d
    };
    let config = OfflineTtsConfig { model: model_config, ..Default::default() };
    match OfflineTts::create(&config) {
        Some(t) => Some(t),
        None => {
            report(format!("model not found — is it at {dir}?"));
            None
        }
    }
}
