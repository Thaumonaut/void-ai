//! On-device STT via sherpa-onnx OfflineRecognizer + cpal mic capture.
//! Push-to-talk: Start begins recording, Stop transcribes the captured audio.
//! Two engines to compare on old hardware:
//!   engine 0 = Moonshine tiny (English only; variable-length → fast on short audio)
//!   engine 1 = Whisper tiny (multilingual incl. Indonesian; fixed 30s window →
//!              compute is ~constant regardless of utterance length)
//! Runs on a dedicated thread; results come back via a callback. Reloading the
//! recognizer is the slow part, so it's cached by engine.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub const STT_MOONSHINE_EN: u8 = 0;
pub const STT_WHISPER_MULTI: u8 = 1;
pub const STT_QWEN3_MULTI: u8 = 2;
pub const STT_OMNI_MULTI: u8 = 3;
pub const STT_SONIOX: u8 = 4; // cloud (Soniox stt-rt-v5), ID+EN code-switch
pub const STT_NATIVE: u8 = 5; // Android's built-in SpeechRecognizer (on-device, free)
pub const STT_DOLPHIN: u8 = 6; // DataoceanAI Dolphin base CTC (on-device; excellent ID, EN empty)

pub enum Cmd {
    /// Begin capturing. `engine` matters only for STT_NATIVE, which drives the
    /// Android SpeechRecognizer directly instead of the cpal mic buffer.
    Start { engine: u8 },
    /// Stop recording and transcribe with the given engine.
    Stop { engine: u8 },
    /// Stop recording and transcribe the SAME audio with every engine (compare).
    CompareStop,
}

/// Engines run in compare mode (the on-device + cloud sherpa-buffer engines;
/// Native isn't here — it captures the mic itself, not the shared buffer).
pub const COMPARE_ENGINES: [u8; 6] = [
    STT_MOONSHINE_EN,
    STT_WHISPER_MULTI,
    STT_QWEN3_MULTI,
    STT_OMNI_MULTI,
    STT_DOLPHIN,
    STT_SONIOX,
];

/// Result of a transcription, delivered to the app.
pub struct SttResult {
    pub text: String,
    pub audio_secs: f32,
    pub decode_secs: f32,
    pub engine: &'static str,
    /// Non-empty when the engine failed (native STT self-completes, so it reports
    /// errors through here rather than only via a status message).
    pub err: String,
}

pub struct Stt {
    tx: Sender<Cmd>,
}

impl Stt {
    pub fn new(
        status: Box<dyn Fn(String) + Send>,
        on_text: Box<dyn Fn(SttResult) + Send>,
        on_compare: Box<dyn Fn(SttResult) + Send>,
    ) -> Self {
        let (tx, rx) = channel::<Cmd>();
        std::thread::spawn(move || stt_thread(rx, status, on_text, on_compare));
        Self { tx }
    }
    pub fn start(&self, engine: u8) {
        let _ = self.tx.send(Cmd::Start { engine });
    }
    pub fn stop(&self, engine: u8) {
        let _ = self.tx.send(Cmd::Stop { engine });
    }
    pub fn compare_stop(&self) {
        let _ = self.tx.send(Cmd::CompareStop);
    }
}

fn engine_name(engine: u8) -> &'static str {
    match engine {
        STT_WHISPER_MULTI => "Whisper-multi",
        STT_QWEN3_MULTI => "Qwen3-multi",
        STT_OMNI_MULTI => "Omni-CTC",
        STT_SONIOX => "Soniox",
        STT_NATIVE => "Native (Android)",
        STT_DOLPHIN => "Dolphin",
        _ => "Moonshine-en",
    }
}

/// BCP-47 locale for the Android SpeechRecognizer. Defaults to English (the
/// user tests in English); drop `/sdcard/kaira/native_locale.txt` with e.g.
/// `id-ID` to hear Indonesian recognition instead.
#[cfg(target_os = "android")]
fn native_locale() -> String {
    std::fs::read_to_string(format!("{}/native_locale.txt", crate::tts::files_dir()))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "en-US".to_string())
}

fn stt_model_dir(engine: u8) -> String {
    let name = match engine {
        STT_WHISPER_MULTI => "sherpa-onnx-whisper-tiny",
        STT_QWEN3_MULTI => "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25",
        STT_OMNI_MULTI => "sherpa-onnx-omnilingual-asr-1600-languages-300M-ctc-v2-int8-2026-02-05",
        STT_DOLPHIN => "sherpa-onnx-dolphin-base-ctc-multi-lang-int8-2025-04-02",
        _ => "sherpa-onnx-moonshine-tiny-en-quantized-2026-02-27",
    };
    if let Ok(base) = std::env::var("KAIRA_STT_DIR") {
        return format!("{base}/{name}");
    }
    #[cfg(target_os = "android")]
    {
        format!("/sdcard/kaira/{name}")
    }
    #[cfg(not(target_os = "android"))]
    {
        format!("/Users/jek/Documents/Projects/Personal/Rust-Mobile/voicelab/{name}")
    }
}

fn stt_thread(
    rx: std::sync::mpsc::Receiver<Cmd>,
    status: Box<dyn Fn(String) + Send>,
    on_text: Box<dyn Fn(SttResult) + Send>,
    on_compare: Box<dyn Fn(SttResult) + Send>,
) {
    // Capture state: the live input stream + the shared sample buffer + its rate.
    let mut stream: Option<cpal::Stream> = None;
    let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let mut capture_rate: u32 = 16000;
    let mut rt: Option<tokio::runtime::Runtime> = None; // lazy tokio for Soniox cloud STT
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Start { engine } => {
                #[cfg(target_os = "android")]
                if engine == STT_NATIVE {
                    if !crate::native_stt::available() {
                        status("Native STT unavailable on this device".to_string());
                        continue;
                    }
                    let lang = native_locale();
                    match crate::native_stt::start(&lang) {
                        Ok(()) => {
                            status(format!("● listening (native · {lang} · on-device)… speak now"));
                            // The recognizer endpoints on its own — poll until it's
                            // done (Stop just force-finalizes early) and deliver the
                            // result, so the user's Stop tap timing doesn't matter.
                            let r = native_session(&rx, &status);
                            on_text(r);
                        }
                        Err(e) => status(format!("native STT: {e}")),
                    }
                    continue;
                }
                let _ = engine; // only STT_NATIVE varies the Start path
                buffer.lock().unwrap().clear();
                match start_capture(buffer.clone()) {
                    Ok((s, rate)) => {
                        capture_rate = rate;
                        stream = Some(s);
                        status("● listening… (tap Stop)".to_string());
                    }
                    Err(e) => status(format!("mic error: {e}")),
                }
            }
            Cmd::Stop { engine } => {
                // Native self-completes in the Start handler; a late Stop is a no-op.
                #[cfg(target_os = "android")]
                if engine == STT_NATIVE {
                    continue;
                }
                stream = None; // dropping the stream stops capture
                let samples = std::mem::take(&mut *buffer.lock().unwrap());
                if samples.is_empty() {
                    status("no audio captured".to_string());
                    continue;
                }
                status(format!("{} · transcribing…", engine_name(engine)));
                if let Some(r) = transcribe_one(engine, &samples, capture_rate, &mut rt, &status) {
                    on_text(r);
                }
            }
            Cmd::CompareStop => {
                stream = None;
                let samples = std::mem::take(&mut *buffer.lock().unwrap());
                if samples.is_empty() {
                    status("no audio captured".to_string());
                    continue;
                }
                let secs = samples.len() as f32 / capture_rate as f32;
                for (i, &engine) in COMPARE_ENGINES.iter().enumerate() {
                    status(format!(
                        "compare [{}/{}] {} …",
                        i + 1,
                        COMPARE_ENGINES.len(),
                        engine_name(engine)
                    ));
                    if let Some(r) = transcribe_one(engine, &samples, capture_rate, &mut rt, &status) {
                        on_compare(r); // stream each result to the UI as it finishes
                    }
                }
                status(format!("compare done · {secs:.1}s clip"));
            }
        }
    }
}

/// Transcribe `samples` with ONE engine (on-device or Soniox cloud), loading a
/// fresh recognizer each call — so compare frees each model's RAM before the next.
fn transcribe_one(
    engine: u8,
    samples: &[f32],
    rate: u32,
    rt: &mut Option<tokio::runtime::Runtime>,
    status: &dyn Fn(String),
) -> Option<SttResult> {
    let audio_secs = samples.len() as f32 / rate as f32;
    let t0 = std::time::Instant::now();
    if engine == STT_SONIOX {
        if rt.is_none() {
            *rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok();
        }
        let runtime = rt.as_ref()?;
        // Fetch a short-lived Soniox key from the lean server (no long-lived key
        // on the device), then stream the audio straight to Soniox.
        let lean = crate::tts::lean_server();
        let result = runtime.block_on(async {
            match tokio::time::timeout(std::time::Duration::from_secs(30), async {
                let key = crate::soniox::fetch_token(&lean).await?;
                crate::soniox::transcribe(&key, samples, rate).await
            })
            .await
            {
                Ok(r) => r,
                Err(_) => Err("timed out (30s) — slow network to lean/Soniox".to_string()),
            }
        });
        match result {
            Ok(text) => Some(SttResult {
                text,
                audio_secs,
                decode_secs: t0.elapsed().as_secs_f32(),
                engine: engine_name(engine),
                err: String::new(),
            }),
            Err(e) => {
                status(format!("Soniox error: {e}"));
                None
            }
        }
    } else {
        let dir = stt_model_dir(engine);
        let rec = create_recognizer(engine, &dir, status)?;
        let stream = rec.create_stream();
        stream.accept_waveform(rate as i32, samples);
        rec.decode(&stream);
        let text = stream
            .get_result()
            .map(|r| r.text)
            .unwrap_or_default()
            .trim()
            .to_string();
        Some(SttResult {
            text,
            audio_secs,
            decode_secs: t0.elapsed().as_secs_f32(),
            engine: engine_name(engine),
            err: String::new(),
        })
    }
}

/// Drive one Android SpeechRecognizer session to completion. The recognizer
/// endpoints on its own (fires onResults/onError when the speaker stops), so we
/// poll for that instead of waiting on the user's Stop tap — which decouples the
/// result from Stop timing. A Stop command that arrives mid-session just
/// force-finalizes early (stopListening); anything else abandons the session.
/// Always returns an SttResult (with `err` set on failure) so the caller can
/// reset the UI regardless of outcome.
#[cfg(target_os = "android")]
fn native_session(rx: &std::sync::mpsc::Receiver<Cmd>, status: &dyn Fn(String)) -> SttResult {
    use std::sync::mpsc::TryRecvError;
    let t0 = std::time::Instant::now();
    let mut announced_ready = false;
    let fail = |err: String, secs: f32| SttResult {
        text: String::new(),
        audio_secs: secs,
        decode_secs: secs,
        engine: engine_name(STT_NATIVE),
        err,
    };
    loop {
        // Tell the user the exact moment the recognizer is ready (early speech
        // before onReadyForSpeech is dropped, a common cause of "no speech").
        if !announced_ready && crate::native_stt::listening() {
            announced_ready = true;
            status("🎤 speak now (native)…".to_string());
        }

        // The instant the recognizer's VAD says you stopped talking, submit the
        // partial — so the LLM starts BEFORE the finalize pass or a Stop tap
        // (overlaps the STT tail with the LLM). Falls through if no partial yet.
        if crate::native_stt::speech_ended() {
            let p = crate::native_stt::partial();
            if !p.trim().is_empty() {
                crate::native_stt::stop();
                let secs = t0.elapsed().as_secs_f32();
                return SttResult {
                    text: p.trim().to_string(),
                    audio_secs: secs,
                    decode_secs: secs,
                    engine: engine_name(STT_NATIVE),
                    err: String::new(),
                };
            }
        }

        match rx.try_recv() {
            Ok(Cmd::Stop { .. }) => {
                crate::native_stt::stop();
                // Use the latest partial immediately instead of waiting for the
                // recognizer to finalize (skips ~0.3–0.8s). Fall through to polling
                // only if no partial has arrived yet.
                let p = crate::native_stt::partial();
                if !p.trim().is_empty() {
                    let secs = t0.elapsed().as_secs_f32();
                    return SttResult {
                        text: p.trim().to_string(),
                        audio_secs: secs,
                        decode_secs: secs,
                        engine: engine_name(STT_NATIVE),
                        err: String::new(),
                    };
                }
                status("Native STT · finalizing…".to_string());
            }
            Ok(_) => {
                // A new Start/CompareStop supersedes this session — abandon it.
                crate::native_stt::stop();
                return fail(String::new(), t0.elapsed().as_secs_f32());
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                crate::native_stt::stop();
                return fail("disconnected".to_string(), t0.elapsed().as_secs_f32());
            }
        }
        let (done, result, error) = crate::native_stt::poll();
        if done {
            let secs = t0.elapsed().as_secs_f32();
            if !error.is_empty() {
                return fail(error, secs);
            }
            return SttResult {
                text: result.trim().to_string(),
                audio_secs: secs,
                decode_secs: secs,
                engine: engine_name(STT_NATIVE),
                err: String::new(),
            };
        }
        if t0.elapsed().as_secs_f32() > 20.0 {
            crate::native_stt::stop();
            return fail("timed out (20s) — no speech detected".to_string(), 20.0);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Build + start a mono f32 mic capture into `buffer`. Returns the stream (keep
/// it alive to keep recording) and the capture sample rate.
fn start_capture(buffer: Arc<Mutex<Vec<f32>>>) -> Result<(cpal::Stream, u32), String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no input device".to_string())?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("no input config: {e}"))?;
    let rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.config();

    let err_fn = |e| eprintln!("[stt] stream error: {e}");
    let buf = buffer;

    macro_rules! build {
        ($t:ty, $to_f32:expr) => {{
            let buf = buf.clone();
            device
                .build_input_stream(
                    &config,
                    move |data: &[$t], _: &cpal::InputCallbackInfo| {
                        let mut b = buf.lock().unwrap();
                        if channels <= 1 {
                            for &s in data {
                                b.push($to_f32(s));
                            }
                        } else {
                            // downmix: keep channel 0 of each frame
                            for frame in data.chunks(channels) {
                                b.push($to_f32(frame[0]));
                            }
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("build stream: {e}"))?
        }};
    }

    let stream = match sample_format {
        cpal::SampleFormat::F32 => build!(f32, |s: f32| s),
        cpal::SampleFormat::I16 => build!(i16, |s: i16| s as f32 / 32768.0),
        cpal::SampleFormat::U16 => build!(u16, |s: u16| (s as f32 - 32768.0) / 32768.0),
        other => return Err(format!("unsupported sample format: {other:?}")),
    };
    stream.play().map_err(|e| format!("play: {e}"))?;
    Ok((stream, rate))
}

fn create_recognizer(
    engine: u8,
    dir: &str,
    status: &dyn Fn(String),
) -> Option<sherpa_onnx::OfflineRecognizer> {
    use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};
    let mut cfg = OfflineRecognizerConfig::default();
    match engine {
        STT_WHISPER_MULTI => {
            // Multilingual Whisper-tiny (int8). language = None → auto-detect per
            // utterance (handles ID or EN); task = transcribe (not translate).
            cfg.model_config.whisper.encoder = Some(format!("{dir}/tiny-encoder.int8.onnx"));
            cfg.model_config.whisper.decoder = Some(format!("{dir}/tiny-decoder.int8.onnx"));
            cfg.model_config.whisper.task = Some("transcribe".to_string());
            cfg.model_config.tokens = Some(format!("{dir}/tiny-tokens.txt"));
        }
        STT_QWEN3_MULTI => {
            // Qwen3-ASR 0.6B (int8). Autoregressive decoder — much stronger on
            // code-switch/multilingual than tiny models, but far heavier. The
            // `tokenizer` field is a DIRECTORY (merges.txt + vocab.json), not a
            // tokens file, so `model_config.tokens` stays unset here.
            cfg.model_config.qwen3_asr.conv_frontend = Some(format!("{dir}/conv_frontend.onnx"));
            cfg.model_config.qwen3_asr.encoder = Some(format!("{dir}/encoder.int8.onnx"));
            cfg.model_config.qwen3_asr.decoder = Some(format!("{dir}/decoder.int8.onnx"));
            cfg.model_config.qwen3_asr.tokenizer = Some(format!("{dir}/tokenizer"));
        }
        STT_OMNI_MULTI => {
            // Meta Omnilingual ASR 300M, CTC (single-pass — no decoder loop, so
            // decode scales linearly with audio). Best on-device ID accuracy per
            // MB. Just a single model file + tokens.
            cfg.model_config.omnilingual.model = Some(format!("{dir}/model.int8.onnx"));
            cfg.model_config.tokens = Some(format!("{dir}/tokens.txt"));
        }
        STT_DOLPHIN => {
            // DataoceanAI Dolphin base CTC (int8, ~104 MB) — Eastern-language ASR.
            // Blazing fast (single-pass CTC, RTF ~0.01) and excellent on Indonesian,
            // but returns EMPTY on English. Single model file + tokens.
            cfg.model_config.dolphin.model = Some(format!("{dir}/model.int8.onnx"));
            cfg.model_config.tokens = Some(format!("{dir}/tokens.txt"));
        }
        _ => {
            cfg.model_config.moonshine.encoder = Some(format!("{dir}/encoder_model.ort"));
            cfg.model_config.moonshine.merged_decoder =
                Some(format!("{dir}/decoder_model_merged.ort"));
            cfg.model_config.tokens = Some(format!("{dir}/tokens.txt"));
        }
    }
    cfg.model_config.provider = Some("cpu".to_string());
    cfg.model_config.num_threads = 4;
    cfg.model_config.debug = false;
    match OfflineRecognizer::create(&cfg) {
        Some(r) => Some(r),
        None => {
            status(format!("{} not found — is it at {dir}?", engine_name(engine)));
            None
        }
    }
}
