//! Host sanity check for the Qwen3-ASR 0.6B (int8) STT path used by the
//! kaira-slint harness. Reads a wav, transcribes it with the SAME sherpa-onnx
//! OfflineRecognizer qwen3_asr config the harness uses, and prints text + RTF.
//!
//!   cargo run --example qwen_check --release -- [MODEL_DIR] [WAV]
//!
//! Default wav = the model's test_wavs/codeswitch.wav (EN/FR/IT/ES in one clip).
//! NOTE: RTF here is host CPU — NOT representative of the A3's Cortex-A53. This
//! proves the qwen3_asr API usage is correct and the model transcribes.

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25".to_string());
    let wav = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| format!("{dir}/test_wavs/codeswitch.wav"));

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

    // --- same config as kaira-slint stt.rs (STT_QWEN3_MULTI) ---
    let mut cfg = OfflineRecognizerConfig::default();
    cfg.model_config.qwen3_asr.conv_frontend = Some(format!("{dir}/conv_frontend.onnx"));
    cfg.model_config.qwen3_asr.encoder = Some(format!("{dir}/encoder.int8.onnx"));
    cfg.model_config.qwen3_asr.decoder = Some(format!("{dir}/decoder.int8.onnx"));
    cfg.model_config.qwen3_asr.tokenizer = Some(format!("{dir}/tokenizer"));
    cfg.model_config.provider = Some("cpu".to_string());
    cfg.model_config.num_threads = 4;

    let load0 = Instant::now();
    let rec = OfflineRecognizer::create(&cfg).expect("create qwen3-asr recognizer");
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
