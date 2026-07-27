//! Unified on-device ASR benchmark: run any supported engine over a wav and
//! report load/decode/RTF + the transcription. Used to compare on-device STT
//! engines on the SAME audio, on host and on real Android hardware.
//!
//!   cargo run --example asr_bench --release -- <engine> <model_dir> <wav> [threads]
//!
//! engine ∈ { moonshine | whisper | qwen3 }
//! (whisper file prefix tiny/base/small is inferred from the model dir name.)

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};
use std::time::Instant;

fn read_wav(path: &str) -> (Vec<f32>, i32) {
    let mut reader = hound::WavReader::open(path).expect("open wav");
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
    let mono = if ch <= 1 {
        interleaved
    } else {
        interleaved.chunks(ch).map(|f| f[0]).collect()
    };
    (mono, rate)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let engine = a.get(1).map(|s| s.as_str()).unwrap_or("whisper");
    let dir = a.get(2).cloned().expect("need <model_dir>");
    let wav = a.get(3).cloned().expect("need <wav>");
    let threads: i32 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(4);

    let (mono, rate) = read_wav(&wav);
    let audio_secs = mono.len() as f32 / rate as f32;

    let mut cfg = OfflineRecognizerConfig::default();
    match engine {
        "moonshine" => {
            cfg.model_config.moonshine.encoder = Some(format!("{dir}/encoder_model.ort"));
            cfg.model_config.moonshine.merged_decoder =
                Some(format!("{dir}/decoder_model_merged.ort"));
            cfg.model_config.tokens = Some(format!("{dir}/tokens.txt"));
        }
        "whisper" => {
            let p = if dir.contains("base") {
                "base"
            } else if dir.contains("small") {
                "small"
            } else {
                "tiny"
            };
            cfg.model_config.whisper.encoder = Some(format!("{dir}/{p}-encoder.int8.onnx"));
            cfg.model_config.whisper.decoder = Some(format!("{dir}/{p}-decoder.int8.onnx"));
            cfg.model_config.whisper.task = Some("transcribe".to_string());
            cfg.model_config.tokens = Some(format!("{dir}/{p}-tokens.txt"));
        }
        "qwen3" => {
            cfg.model_config.qwen3_asr.conv_frontend = Some(format!("{dir}/conv_frontend.onnx"));
            cfg.model_config.qwen3_asr.encoder = Some(format!("{dir}/encoder.int8.onnx"));
            cfg.model_config.qwen3_asr.decoder = Some(format!("{dir}/decoder.int8.onnx"));
            cfg.model_config.qwen3_asr.tokenizer = Some(format!("{dir}/tokenizer"));
        }
        "omni" => {
            // Meta Omnilingual ASR, CTC (single-pass — no decoder loop).
            cfg.model_config.omnilingual.model = Some(format!("{dir}/model.int8.onnx"));
            cfg.model_config.tokens = Some(format!("{dir}/tokens.txt"));
        }
        "dolphin" => {
            // DataoceanAI Dolphin, CTC. Native ID text output, ~104MB int8.
            cfg.model_config.dolphin.model = Some(format!("{dir}/model.int8.onnx"));
            cfg.model_config.tokens = Some(format!("{dir}/tokens.txt"));
        }
        other => panic!("unknown engine: {other} (use moonshine|whisper|qwen3|omni|dolphin)"),
    }
    cfg.model_config.provider = Some("cpu".to_string());
    cfg.model_config.num_threads = threads;

    let l0 = Instant::now();
    let rec = OfflineRecognizer::create(&cfg).expect("create recognizer");
    let load = l0.elapsed().as_secs_f32();

    let t0 = Instant::now();
    let stream = rec.create_stream();
    stream.accept_waveform(rate, &mono);
    rec.decode(&stream);
    let text = stream.get_result().map(|r| r.text).unwrap_or_default();
    let decode = t0.elapsed().as_secs_f32();

    // one machine-parseable line + the text
    println!(
        "RESULT engine={engine} threads={threads} audio={audio_secs:.2} load={load:.2} decode={decode:.2} rtf={:.3}",
        decode / audio_secs.max(0.001)
    );
    println!("TEXT {}", text.trim());
}
