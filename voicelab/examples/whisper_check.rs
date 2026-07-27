//! Host sanity check for the multilingual Whisper-tiny STT path used by the
//! kaira-slint harness. Reads a wav, transcribes it with the SAME sherpa-onnx
//! OfflineRecognizer whisper config the harness uses, and prints text + RTF.
//!
//!   cargo run --example whisper_check --release -- [MODEL_DIR] [WAV]
//!
//! Defaults: model dir = sherpa-onnx-whisper-tiny, wav = its test_wavs/0.wav.
//! NOTE: RTF here is on the host CPU — NOT representative of the A3's Cortex-A53.
//! This only proves the whisper API usage is correct and the model transcribes.

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "sherpa-onnx-whisper-tiny".to_string());
    let wav = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| format!("{dir}/test_wavs/0.wav"));

    // --- read wav -> mono f32 at its native sample rate ---
    let mut reader = hound::WavReader::open(&wav).expect("open wav");
    let spec = reader.spec();
    let rate = spec.sample_rate as i32;
    let ch = spec.channels as usize;
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.unwrap() as f32 / max)
                .collect()
        }
    };
    let mono: Vec<f32> = if ch <= 1 {
        interleaved
    } else {
        interleaved.chunks(ch).map(|f| f[0]).collect()
    };
    let audio_secs = mono.len() as f32 / rate as f32;

    // --- same config as kaira-slint stt.rs (STT_WHISPER_MULTI) ---
    let mut cfg = OfflineRecognizerConfig::default();
    cfg.model_config.whisper.encoder = Some(format!("{dir}/tiny-encoder.int8.onnx"));
    cfg.model_config.whisper.decoder = Some(format!("{dir}/tiny-decoder.int8.onnx"));
    cfg.model_config.whisper.task = Some("transcribe".to_string());
    cfg.model_config.tokens = Some(format!("{dir}/tiny-tokens.txt"));
    cfg.model_config.provider = Some("cpu".to_string());
    cfg.model_config.num_threads = 4;

    let load0 = Instant::now();
    let rec = OfflineRecognizer::create(&cfg).expect("create whisper recognizer");
    let load_secs = load0.elapsed().as_secs_f32();

    let t0 = Instant::now();
    let stream = rec.create_stream();
    stream.accept_waveform(rate, &mono);
    rec.decode(&stream);
    let text = stream.get_result().map(|r| r.text).unwrap_or_default();
    let decode_secs = t0.elapsed().as_secs_f32();

    println!("wav       : {wav}");
    println!("rate/ch   : {rate} Hz, {ch} ch");
    println!("audio     : {audio_secs:.2}s");
    println!("load      : {load_secs:.2}s (one-time)");
    println!(
        "decode    : {decode_secs:.2}s   RTF {:.3}",
        decode_secs / audio_secs.max(0.001)
    );
    println!("text      : {}", text.trim());
}
