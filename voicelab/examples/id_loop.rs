//! Real on-device Indonesian round-trip: synthesize an ID sentence with the
//! Piper VITS id_ID model, then transcribe it back with multilingual
//! Whisper-tiny. Proves Whisper (unlike Moonshine-en) handles Bahasa Indonesia.
//!
//!   cargo run --example id_loop --release -- ["some Indonesian text"]
//!
//! RTF numbers are host-CPU, NOT representative of the A3's Cortex-A53.

use sherpa_onnx::{
    GenerationConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTts, OfflineTtsConfig,
    OfflineTtsModelConfig, OfflineTtsVitsModelConfig,
};
use std::time::Instant;

fn main() {
    let text = std::env::args().nth(1).unwrap_or_else(|| {
        "Halo, saya Kaira. Ini adalah suara yang berjalan langsung di perangkat Anda.".to_string()
    });

    // 1) Synthesize Indonesian with Piper VITS.
    let piper = "vits-piper-id_ID-news_tts-medium-int8";
    let tts_cfg = OfflineTtsConfig {
        model: OfflineTtsModelConfig {
            vits: OfflineTtsVitsModelConfig {
                model: Some(format!("{piper}/id_ID-news_tts-medium.onnx")),
                tokens: Some(format!("{piper}/tokens.txt")),
                data_dir: Some(format!("{piper}/espeak-ng-data")),
                ..Default::default()
            },
            num_threads: 4,
            ..Default::default()
        },
        ..Default::default()
    };
    let tts = OfflineTts::create(&tts_cfg).expect("create piper id");
    let audio = tts
        .generate_with_config(
            &text,
            &GenerationConfig { sid: 0, speed: 1.0, ..Default::default() },
            None::<fn(&[f32], f32) -> bool>,
        )
        .expect("piper synth failed");
    let samples = audio.samples().to_vec();
    let rate = audio.sample_rate();
    let audio_secs = samples.len() as f32 / rate as f32;
    println!("spoke (ID): {audio_secs:.2}s @ {rate}Hz");
    println!("  text    : {text}");

    // 2) Transcribe with multilingual Whisper-tiny (language auto-detect).
    let dir = "sherpa-onnx-whisper-tiny";
    let mut cfg = OfflineRecognizerConfig::default();
    cfg.model_config.whisper.encoder = Some(format!("{dir}/tiny-encoder.int8.onnx"));
    cfg.model_config.whisper.decoder = Some(format!("{dir}/tiny-decoder.int8.onnx"));
    cfg.model_config.whisper.task = Some("transcribe".to_string());
    cfg.model_config.tokens = Some(format!("{dir}/tiny-tokens.txt"));
    cfg.model_config.provider = Some("cpu".to_string());
    cfg.model_config.num_threads = 4;
    let rec = OfflineRecognizer::create(&cfg).expect("create whisper");

    let t0 = Instant::now();
    let stream = rec.create_stream();
    stream.accept_waveform(rate as i32, &samples);
    rec.decode(&stream);
    let heard = stream.get_result().map(|r| r.text).unwrap_or_default();
    let decode_secs = t0.elapsed().as_secs_f32();

    println!(
        "heard (Whisper): decode={decode_secs:.2}s  RTF {:.3}",
        decode_secs / audio_secs.max(0.001)
    );
    println!("  text    : {}", heard.trim());
}
